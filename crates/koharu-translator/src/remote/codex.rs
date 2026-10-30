use anyhow::{Context as _, ensure};
use koharu_codex::{Codex, CodexModel, Control, Reasoning, Request};
use serde_json::json;
use uuid::Uuid;

use crate::{
    GenerationConfig, Model, Provider, Result, TranslationRequest, backend::encode_image, prompt,
};

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(default)]
pub struct CodexConfig {}

pub(super) async fn models() -> Result<Vec<Model>> {
    let codex = Codex::new()?;
    if codex.account()?.is_none() {
        return Ok(Vec::new());
    }
    let models = tokio::time::timeout(std::time::Duration::from_secs(5), codex.models())
        .await
        .context("Codex model discovery timed out")??;
    Ok(models
        .into_iter()
        .map(|model| Model {
            provider: Provider::Codex,
            model: Some(model.id),
            name: model.name,
            quantizations: Vec::new(),
            vision: model.vision,
            reasoning: model
                .reasoning
                .iter()
                .any(|level| *level != Reasoning::None),
        })
        .collect())
}

pub(super) async fn translate(
    model: &str,
    generation: &GenerationConfig,
    request: &TranslationRequest,
) -> anyhow::Result<Vec<String>> {
    let codex = Codex::new()?;
    ensure!(
        codex.account()?.is_some(),
        "Sign in with Codex in the Agent panel before using Codex translation"
    );
    let models = codex.models().await?;
    let selected = models
        .iter()
        .find(|candidate| candidate.id == model)
        .with_context(|| format!("selected Codex translation model {model} is not available"))?;
    let response = codex
        .respond(
            &translation_request(selected, generation, request)?,
            &Control::default(),
            |_| {},
        )
        .await?;
    ensure!(
        response.calls.is_empty(),
        "Codex translation returned unexpected tool calls"
    );
    translations(&response.text, &request.segments)
}

fn translation_request(
    model: &CodexModel,
    generation: &GenerationConfig,
    request: &TranslationRequest,
) -> anyhow::Result<Request> {
    let mut request = request.clone();
    if !model.vision || !generation.vision.unwrap_or(false) {
        request.remove_image();
    }
    let (system, user) = prompt::prompts(&request)?;
    let mut content = vec![json!({"type": "input_text", "text": user})];
    if let Some(image) = request.image.as_deref() {
        content.push(json!({"type": "input_image", "image_url": encode_image(image)?.data_url()}));
    }
    // Codex accepts only its advertised reasoning levels. With reasoning off,
    // choose the lowest supported effort; not every model can disable it.
    let reasoning = if generation.reasoning.unwrap_or(false) {
        model
            .reasoning
            .iter()
            .find(|&&level| level == Reasoning::Medium)
            .or_else(|| {
                model
                    .reasoning
                    .iter()
                    .filter(|&&level| level != Reasoning::None)
                    .min()
            })
            .or_else(|| model.reasoning.iter().min())
    } else {
        model.reasoning.iter().min()
    }
    .copied();
    Ok(Request::new(
        model.id.clone(),
        system,
        vec![json!({"type": "message", "role": "user", "content": content})],
        Vec::new(),
        reasoning,
        Uuid::new_v4().to_string(),
    )
    .with_output_schema(prompt::output_schema(request.segments.len())))
}

fn translations(text: &str, source: &[String]) -> anyhow::Result<Vec<String>> {
    let output: prompt::TranslationOutput = serde_json::from_str(text).with_context(|| {
        format!(
            "Codex returned invalid translation JSON: {}",
            prompt::snippet(text)
        )
    })?;
    ensure!(
        output.translations.len() == source.len(),
        "Codex returned {} translations; expected {}",
        output.translations.len(),
        source.len()
    );
    let mut translated = vec![None; source.len()];
    for segment in output.translations {
        let slot = translated.get_mut(segment.id).with_context(|| {
            format!("Codex returned unknown translation segment {}", segment.id)
        })?;
        ensure!(
            slot.is_none(),
            "Codex returned duplicate translation segment {}",
            segment.id
        );
        *slot = Some(segment.text);
    }
    translated
        .into_iter()
        .enumerate()
        .map(|(id, text)| text.with_context(|| format!("Codex omitted translation segment {id}")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translation_requests_contain_only_page_context_and_supported_options() {
        let mut model = CodexModel {
            id: "codex".to_owned(),
            name: "Codex".to_owned(),
            reasoning: vec![Reasoning::Medium, Reasoning::Low],
            vision: true,
        };
        let request = TranslationRequest::new(["こんにちは"], crate::Language::English)
            .with_instructions("Keep honorifics.")
            .with_image(std::sync::Arc::new(image::DynamicImage::new_rgb8(1, 1)));
        let mut generation = GenerationConfig::default();
        let value =
            serde_json::to_value(translation_request(&model, &generation, &request).unwrap())
                .unwrap();
        assert!(
            value["instructions"]
                .as_str()
                .unwrap()
                .contains("Keep honorifics.")
        );
        assert!(
            value["instructions"]
                .as_str()
                .unwrap()
                .contains("natural English")
        );
        assert_eq!(value["input"].as_array().unwrap().len(), 1);
        let input: serde_json::Value =
            serde_json::from_str(value["input"][0]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(input["segments"][0]["text"], "こんにちは");
        assert_eq!(value["input"][0]["content"][1]["type"], "input_image");
        assert_eq!(value["tool_choice"], "none");
        assert_eq!(value["tools"], json!([]));
        assert_eq!(value["reasoning"]["effort"], "low");
        assert_eq!(value["text"]["format"]["type"], "json_schema");
        assert_eq!(value["text"]["format"]["strict"], true);
        assert_eq!(
            value["text"]["format"]["schema"]["properties"]["translations"]["maxItems"],
            1
        );
        assert!(value.get("temperature").is_none());
        assert!(value.get("max_output_tokens").is_none());

        generation.reasoning = Some(true);
        generation.vision = Some(false);
        let value =
            serde_json::to_value(translation_request(&model, &generation, &request).unwrap())
                .unwrap();
        assert_eq!(value["reasoning"]["effort"], "medium");
        assert_eq!(value["input"][0]["content"].as_array().unwrap().len(), 1);
        assert!(
            !value["instructions"]
                .as_str()
                .unwrap()
                .contains("Image requirements")
        );

        model.reasoning.push(Reasoning::None);
        generation.reasoning = Some(false);
        let value =
            serde_json::to_value(translation_request(&model, &generation, &request).unwrap())
                .unwrap();
        assert_eq!(value["reasoning"]["effort"], "none");

        model.vision = false;
        model.reasoning.clear();
        generation.vision = Some(true);
        let value =
            serde_json::to_value(translation_request(&model, &generation, &request).unwrap())
                .unwrap();
        assert_eq!(value["input"][0]["content"].as_array().unwrap().len(), 1);
        assert!(value.get("reasoning").is_none());
    }

    #[test]
    fn rejects_incomplete_or_ambiguous_translations() {
        let source = ["one".to_owned(), "two".to_owned()];
        for text in [
            r#"{"translations":[{"id":0,"text":"hello"}]}"#,
            r#"{"translations":[{"id":0,"text":"hello"},{"id":0,"text":"world"}]}"#,
            r#"{"translations":[{"id":0,"text":"hello"},{"id":2,"text":"world"}]}"#,
            r#"{"translations":[{"id":0,"text":"hello"},{"id":1,"text":"world"},{"id":2,"text":"extra"}]}"#,
            r#"{"translations":[{"id":-1,"text":"hello"},{"id":1,"text":"world"}]}"#,
            "not JSON",
        ] {
            assert!(translations(text, &source).is_err(), "accepted {text}");
        }
    }

    #[test]
    fn restores_translation_order_from_ids() {
        let source = ["one".to_owned(), "two".to_owned()];
        let response = r#"{"translations":[{"id":1,"text":"world"},{"id":0,"text":"hello"}]}"#;
        assert_eq!(translations(response, &source).unwrap(), ["hello", "world"]);
    }
}
