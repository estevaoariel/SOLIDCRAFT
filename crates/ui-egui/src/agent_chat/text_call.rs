//! Recognise one intended call in model text; execution stays in the normal tool loop.

use serde_json::{Value, json};

const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_DEPTH: usize = 64;

pub(super) fn pickup(text: &str, tools: &[Value]) -> Option<Value> {
    if text.len() > MAX_TEXT_BYTES {
        return None;
    }
    // Scan only a bounded number of candidates, including fenced JSON after prose.
    for (start, _) in text.match_indices('{').take(16) {
        let Some(value) = object(text.get(start..)?) else { continue };
        let Some((name, args)) = intended_call(&value, tools) else { continue };
        let arguments = if args.is_string() { args } else { Value::String(args.to_string()) };
        return Some(json!({
            "id": "text-call", "type": "function",
            "function": {"name": name, "arguments": arguments}
        }));
    }
    None
}

fn intended_call(value: &Value, tools: &[Value]) -> Option<(String, Value)> {
    let call = value.get("function").unwrap_or(value);
    let (name, mut args) = if let Some(name) = call.get("name").or_else(|| call.get("tool")).and_then(Value::as_str) {
        let args = call.get("arguments").or_else(|| call.get("parameters")).or_else(|| call.get("args")).cloned().unwrap_or_else(|| json!({}));
        (name, args)
    } else {
        let map = call.as_object()?;
        if map.len() != 1 {
            return None;
        }
        let (name, args) = map.iter().next()?;
        // Bare objects are calls only when their key names a real command.
        let split = name.find(|c: char| c.is_whitespace() || c == '{' || c == '(').unwrap_or(name.len());
        solidcraft_engine::cmd::find_command(name.get(..split)?)?;
        (name.as_str(), args.clone())
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
    // Unknown named tools get the same error feedback as native calls.
    Some((name.to_string(), args))
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
