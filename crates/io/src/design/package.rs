//! Shared authoritative metadata and payload representation for native containers and Git.
//! Storage-specific compression, caching and transport stay outside this module.
use super::*;

pub(crate) fn encode(doc: &Document, compression: DesignCompression, mut blob: impl FnMut(&[u8], &str, bool) -> Result<Value>) -> Result<Vec<u8>> {
    let mut features = Vec::with_capacity(doc.features.len());
    for feature in &doc.features {
        let replacements = match &feature.kind {
            FeatureKind::Import { step, .. } => {
                // STEP is text and shrinks several times: always compressed, so an import as large
                // as MAX_STEP_BYTES still fits in a design file.
                let blob = blob(step.as_bytes(), "step", true)?;
                fields([("step", Value::String(String::new())), ("step_blob", blob)])
            }
            FeatureKind::MeshImport { meshes, .. } => {
                let mut metadata = Vec::with_capacity(meshes.len());
                for mesh in meshes {
                    let bytes = encode_mesh(mesh)?;
                    let blob = blob(&bytes, "mesh", compression == DesignCompression::Compact)?;
                    let mut value = replace(mesh, fields([("positions", json!([])), ("triangles", json!([]))])).map_err(invalid)?;
                    value.as_object_mut().ok_or_else(|| invalid("mesh metadata"))?.insert("mesh_blob".into(), blob);
                    metadata.push(value);
                }
                fields([("meshes", Value::Array(metadata))])
            }
            _ => Map::new(),
        };
        // Extra reference fields are added after serializing existing fields.
        let step_blob = replacements.get("step_blob").cloned();
        let mut value = replace(feature, replacements).map_err(invalid)?;
        if let Some(blob) = step_blob {
            value.as_object_mut().ok_or_else(|| invalid("feature metadata"))?.insert("step_blob".into(), blob);
        }
        features.push(value);
    }
    let mut canvases = Vec::with_capacity(doc.canvases.len());
    for canvas in &doc.canvases {
        if canvas.data.len() > solidcraft_doc::canvas::MAX_CANVAS_BYTES.div_ceil(3) * 4 {
            return Err(invalid("canvas too large"));
        }
        let bytes = canvas.bytes().ok_or_else(|| invalid("invalid canvas base64"))?;
        if !matches!(canvas.format.as_str(), "png" | "jpeg") || bytes.len() > solidcraft_doc::canvas::MAX_CANVAS_BYTES {
            return Err(invalid("invalid canvas format or size"));
        }
        let blob = blob(&bytes, &canvas.format, false)?;
        let mut value = replace(canvas, fields([("data", json!(""))])).map_err(invalid)?;
        value.as_object_mut().ok_or_else(|| invalid("canvas metadata"))?.insert("image_blob".into(), blob);
        canvases.push(value);
    }
    let metadata = replace(doc, fields([("features", Value::Array(features)), ("canvases", Value::Array(canvases))])).map_err(invalid)?;
    serde_json::to_vec(&metadata).map_err(invalid)
}

pub(crate) trait BlobReader {
    fn blob(&mut self, value: &Value, field: &str, extension: &str) -> Result<&[u8]>;
}

pub(crate) fn decode(bytes: &[u8], reader: &mut impl BlobReader) -> Result<Document> {
    let metadata: Value = serde_json::from_slice(bytes).map_err(invalid)?;
    let text = std::str::from_utf8(bytes).map_err(invalid)?;
    // Migrations and all document validation run on the small parametric metadata.
    // Binary arrays have their own equivalent numeric/index validation below.
    let mut doc = Document::from_json(text)?;
    for (index, feature) in doc.features.iter_mut().enumerate() {
        let value = metadata.get("features").and_then(|v| v.get(index)).ok_or_else(|| invalid("missing feature metadata"))?;
        match &mut feature.kind {
            FeatureKind::Import { step, .. } => {
                if !step.is_empty() {
                    return Err(invalid("ambiguous embedded STEP and blob"));
                }
                *step = std::str::from_utf8(reader.blob(value, "step_blob", "step")?).map_err(invalid)?.into();
            }
            FeatureKind::MeshImport { meshes, .. } => {
                for (index, mesh) in meshes.iter_mut().enumerate() {
                    if !mesh.positions.is_empty() || !mesh.triangles.is_empty() {
                        return Err(invalid("ambiguous embedded mesh and blob"));
                    }
                    let value = value.get("meshes").and_then(|v| v.get(index)).ok_or_else(|| invalid("missing mesh metadata"))?;
                    decode_mesh(reader.blob(value, "mesh_blob", "mesh")?, mesh)?;
                }
            }
            _ => {}
        }
    }
    for (index, canvas) in doc.canvases.iter_mut().enumerate() {
        if !canvas.data.is_empty() || !matches!(canvas.format.as_str(), "png" | "jpeg") {
            return Err(invalid("invalid canvas metadata"));
        }
        let value = metadata.get("canvases").and_then(|v| v.get(index)).ok_or_else(|| invalid("missing canvas metadata"))?;
        let bytes = reader.blob(value, "image_blob", &canvas.format)?;
        if bytes.len() > solidcraft_doc::canvas::MAX_CANVAS_BYTES {
            return Err(invalid("canvas too large"));
        }
        canvas.data = solidcraft_doc::canvas::base64_encode(bytes);
    }
    Ok(doc)
}
