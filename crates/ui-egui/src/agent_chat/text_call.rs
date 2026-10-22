//! Recognise one intended call in model text; execution stays in the normal tool loop. Also the
//! narrow checks for a reply that only describes an action instead of calling a tool.

use serde_json::{Value, json};

const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_DEPTH: usize = 64;
/// Words of a reply looked at for a command id, at most.
const MAX_WORDS: usize = 4096;
/// Words of the user's message looked at for a request to change the design, at most.
const MAX_REQUEST_WORDS: usize = 64;

/// One intended call written in the text, as a `tool_calls` entry, and the byte offset where
/// it starts (the prose before it is still the model's answer).
pub(super) fn pickup(text: &str, tools: &[Value]) -> Option<(Value, usize)> {
    if text.len() > MAX_TEXT_BYTES {
        return None;
    }
    // Scan only a bounded number of candidates, including fenced JSON after prose.
    for (start, _) in text.match_indices('{').take(16) {
        let Some(value) = object(text.get(start..)?) else { continue };
        let Some((name, args)) = intended_call(&value, tools) else { continue };
        let arguments = if args.is_string() { args } else { Value::String(args.to_string()) };
        let call = json!({
            "id": "text-call", "type": "function",
            "function": {"name": name, "arguments": arguments}
        });
        return Some((call, start));
    }
    None
}

/// The prose before a picked-up call, without the opening of a code fence around it.
pub(super) fn prose_before(text: &str, start: usize) -> &str {
    let before = text.get(..start).unwrap_or("").trim_end();
    match before.rfind("```") {
        Some(fence) if before.get(fence + 3..).is_some_and(|tag| tag.chars().all(|c| c.is_ascii_alphanumeric())) => {
            before.get(..fence).unwrap_or("").trim_end()
        }
        _ => before,
    }
}

/// The first SolidCraft command id a reply names, in prose or in code (`solid.box`).
pub(super) fn named_command(text: &str) -> Option<&'static str> {
    if text.len() > MAX_TEXT_BYTES {
        return None;
    }
    text.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_')))
        .take(MAX_WORDS)
        .map(|w| w.trim_matches('.'))
        .filter(|w| w.contains('.') && w.len() <= 64)
        .find_map(|w| solidcraft_engine::cmd::find_command(w).map(|spec| spec.id))
}

/// Words that ask for a change to the design.
const CHANGE_WORDS: &str = "add build chamfer change create cut delete draw drill extrude fillet make mirror move pattern remove rename resize \
     revolve rotate scale shell sketch translate";

/// Whether the user's message asks for a change to the design ("make a 20 mm cube").
pub(super) fn asks_for_change(text: &str) -> bool {
    text.split(|c: char| !c.is_alphabetic()).take(MAX_REQUEST_WORDS).any(|w| CHANGE_WORDS.split_whitespace().any(|k| k.eq_ignore_ascii_case(w)))
}

fn intended_call(value: &Value, tools: &[Value]) -> Option<(String, Value)> {
    let call = value.get("function").unwrap_or(value);
    let (name, mut args, has_args) = if let Some(name) = call.get("name").or_else(|| call.get("tool")).and_then(Value::as_str) {
        let given = call.get("arguments").or_else(|| call.get("parameters")).or_else(|| call.get("args"));
        (name, given.cloned().unwrap_or_else(|| json!({})), given.is_some())
    } else {
        let map = call.as_object()?;
        if map.len() != 1 {
            return None;
        }
        let (name, args) = map.iter().next()?;
        // Bare objects are calls only when their key names a real command.
        let split = name.find(|c: char| c.is_whitespace() || c == '{' || c == '(').unwrap_or(name.len());
        solidcraft_engine::cmd::find_command(name.get(..split)?)?;
        (name.as_str(), args.clone(), true)
    };
    if tools.iter().any(|t| t.pointer("/function/name").and_then(Value::as_str) == Some(name)) {
        return Some((name.to_string(), args));
    }
    let split = name.find(|c: char| c.is_whitespace() || c == '{' || c == '(').unwrap_or(name.len());
    let command = name.get(..split)?;
    if let Some(spec) = solidcraft_engine::cmd::find_command(command) {
        if let Some(text) = args.as_str() {
            args = object(text)?;
        }
        let packed = name.get(split..)?.trim();
        if !packed.is_empty() {
            let packed = if let Some(inner) = packed.strip_prefix('(') {
                let inner = inner.strip_suffix(')').unwrap_or(inner).trim();
                if inner.starts_with('{') { inner.to_string() } else { format!("{{{inner}}}") }
            } else {
                packed.to_string()
            };
            let packed = object(&packed)?;
            args.as_object_mut()?.extend(packed.as_object()?.clone());
        }
        return Some(("execute".into(), json!({"command": spec.id, "params": args})));
    }
    // An unknown name with arguments is a call: it gets the same error feedback as a native
    // one. Without arguments it is just an object in the answer (`{"name": "Body1", …}`).
    has_args.then(|| (name.to_string(), args))
}

/// Strip surrounding prose by ending at the root's closing brace. Quote JSON5-style keys
/// and strings, remove trailing commas, and finally try missing closing brackets. No values
/// or command names are guessed. serde_json retains its own recursion limit as well.
fn object(text: &str) -> Option<Value> {
    if text.len() > MAX_TEXT_BYTES {
        return None;
    }
    let mut chars = text.chars().peekable();
    let mut out = String::with_capacity(text.len());
    let mut stack = Vec::new();
    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' => {
                let mut token = String::from("\"");
                let mut closed = false;
                while let Some(ch) = chars.next() {
                    if ch == c {
                        closed = true;
                        break;
                    }
                    match ch {
                        '\\' => {
                            let escaped = chars.next()?;
                            if escaped == '\'' && c == '\'' {
                                token.push('\'');
                            } else {
                                token.push('\\');
                                token.push(escaped);
                            }
                        }
                        '"' => token.push_str("\\\""),
                        _ => token.push(ch),
                    }
                }
                if !closed {
                    return None;
                }
                token.push('"');
                out.push_str(&token);
            }
            '{' | '[' => {
                if stack.len() >= MAX_DEPTH {
                    return None;
                }
                stack.push(if c == '{' { '}' } else { ']' });
                out.push(c);
            }
            '}' | ']' => {
                if stack.pop() != Some(c) {
                    return None;
                }
                trim_comma(&mut out);
                out.push(c);
                if stack.is_empty() {
                    break;
                }
            }
            // A closing fence can follow a call that is missing its final brackets.
            '`' if chars.peek() == Some(&'`') => break,
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut token = String::from(c);
                while chars.peek().is_some_and(|ch| ch.is_ascii_alphanumeric() || matches!(*ch, '_' | '-' | '.')) {
                    token.push(chars.next()?);
                }
                while chars.peek().is_some_and(|ch| ch.is_whitespace()) {
                    chars.next();
                }
                if chars.peek() == Some(&':') {
                    out.push_str(&serde_json::to_string(&token).ok()?);
                } else {
                    out.push_str(&token);
                }
            }
            _ => out.push(c),
        }
    }
    if let Ok(v @ Value::Object(_)) = serde_json::from_str(text) {
        return Some(v);
    }
    if let Ok(v @ Value::Object(_)) = serde_json::from_str(&out) {
        return Some(v);
    }
    trim_comma(&mut out);
    for close in stack.into_iter().rev() {
        out.push(close);
    }
    match serde_json::from_str(&out) {
        Ok(v @ Value::Object(_)) => Some(v),
        _ => None,
    }
}

fn trim_comma(text: &mut String) {
    text.truncate(text.trim_end().len());
    if text.ends_with(',') {
        text.pop();
    }
}
