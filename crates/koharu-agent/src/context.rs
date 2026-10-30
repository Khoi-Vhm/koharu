use serde_json::{Value, json};

pub(crate) fn project_context(data: &Value) -> Result<Value, serde_json::Error> {
    let content = vec![json!({
        "type": "input_text",
        "text": format!(
            "<koharu_project_context>\n{}\n</koharu_project_context>",
            serde_json::to_string(data)?
        ),
    })];
    Ok(json!({
        "type": "message",
        "role": "user",
        "content": content,
    }))
}

pub(crate) fn function_output(
    call_id: &str,
    output: &Value,
    images: &[crate::ToolImage],
) -> Result<Value, serde_json::Error> {
    let output = if images.is_empty() {
        Value::String(serde_json::to_string(output)?)
    } else {
        let mut content = vec![json!({
            "type": "input_text",
            "text": serde_json::to_string(output)?,
        })];
        for image in images {
            content.push(json!({
                "type": "input_text",
                "text": image.label,
            }));
            content.push(json!({
                "type": "input_image",
                "image_url": image.data_url,
                "detail": "high",
            }));
        }
        Value::Array(content)
    };
    Ok(json!({
        "type": "function_call_output",
        "call_id": call_id,
        "output": output,
    }))
}
