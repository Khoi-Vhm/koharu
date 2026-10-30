use std::future::Future;

use anyhow::{Context, ensure};

use crate::{TranslationRequest, backend::TranslationTask};

pub(crate) struct Output {
    pub translations: Vec<String>,
    pub dialogue: Option<String>,
}

pub(crate) async fn translate<F, Fut>(
    mut request: TranslationRequest,
    contextual: bool,
    mut complete: F,
) -> anyhow::Result<Output>
where
    F: FnMut(TranslationRequest) -> Fut,
    Fut: Future<Output = anyhow::Result<Vec<String>>>,
{
    if request.segments.is_empty() {
        return Ok(Output {
            translations: Vec::new(),
            dialogue: None,
        });
    }
    ensure!(
        request.regions.is_empty() || request.regions.len() == request.segments.len(),
        "source regions must match translation segments"
    );
    if !contextual {
        return Ok(Output {
            translations: complete(request).await?,
            dialogue: None,
        });
    }

    // One synthetic segment lets every LLM provider reuse its segment-preserving
    // JSON transport for a page-level analysis without a second provider API.
    let mut analysis = request.clone();
    analysis.segments = vec![serde_json::to_string(&request.segments)?];
    analysis.task = TranslationTask::Analysis;
    let notes = complete(analysis)
        .await
        .context("failed to analyze Vietnamese dialogue")?;
    ensure!(
        notes.len() == 1 && !notes[0].trim().is_empty(),
        "dialogue analysis must contain one nonempty summary"
    );
    ensure!(
        notes[0].len() <= 64_000,
        "dialogue analysis exceeds the memory limit"
    );
    request.dialogue = notes.into_iter().next();
    let draft = complete(request.clone())
        .await
        .context("failed to translate Vietnamese dialogue")?;
    ensure!(
        draft.len() == request.segments.len(),
        "dialogue draft has an invalid segment count"
    );
    request.task = TranslationTask::Review(draft);
    let dialogue = request.dialogue.clone();
    let translations = complete(request)
        .await
        .context("failed to review Vietnamese forms of address")?;
    Ok(Output {
        translations,
        dialogue,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::{Language, TranslationRequest, backend::TranslationTask};

    #[tokio::test]
    async fn analyzes_translates_and_reviews_with_the_same_context() {
        let mut request = TranslationRequest::new(["Where are you?"], Language::Vietnamese)
            .with_image(Arc::new(image::DynamicImage::new_rgb8(1, 1)));
        request.dialogue = Some("A serves B.".to_owned());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let calls = seen.clone();
        let result = super::translate(request, true, move |request| {
            let mut calls = calls.lock().unwrap();
            let output = match calls.len() {
                0 => "A addresses B as ngài; speaker for segment 0 is A.",
                1 => "Ngài ở đâu?",
                2 => "Ngài đang ở đâu?",
                _ => panic!("unexpected model call"),
            };
            calls.push(request);
            std::future::ready(Ok(vec![output.to_owned()]))
        })
        .await
        .unwrap();
        assert_eq!(result.translations, ["Ngài đang ở đâu?"]);
        assert!(result.dialogue.unwrap().contains("ngài"));
        let calls = seen.lock().unwrap();
        assert!(matches!(calls[0].task, TranslationTask::Analysis));
        assert!(matches!(calls[1].task, TranslationTask::Translate));
        assert!(matches!(calls[2].task, TranslationTask::Review(_)));
        assert!(calls.iter().all(|call| call.image.is_some()));
        assert_eq!(calls[1].segments, ["Where are you?"]);
        assert_eq!(calls[1].dialogue, calls[2].dialogue);
    }

    #[tokio::test]
    async fn single_pass_does_not_carry_unverified_memory() {
        let request = TranslationRequest::new(["hello"], Language::English);
        let mut calls = 0;
        let result = super::translate(request, false, |_| {
            calls += 1;
            std::future::ready(Ok(vec!["hello".to_owned()]))
        })
        .await
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(result.translations, ["hello"]);
        assert!(result.dialogue.is_none());
    }

    #[tokio::test]
    async fn rejects_empty_analysis_before_translation() {
        let request = TranslationRequest::new(["hello"], Language::Vietnamese);
        let result = super::translate(request, true, |_| {
            std::future::ready(Ok(vec![String::new()]))
        })
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn empty_input_makes_no_model_calls() {
        let request = TranslationRequest::new(Vec::<String>::new(), Language::Vietnamese);
        let output = super::translate(request, true, |_| async {
            panic!("empty input must not call the model");
            #[allow(unreachable_code)]
            Ok(Vec::new())
        })
        .await
        .unwrap();
        assert!(output.translations.is_empty());
    }

    #[tokio::test]
    async fn model_failure_stops_the_workflow() {
        let request = TranslationRequest::new(["hello"], Language::Vietnamese);
        let mut calls = 0;
        let result = super::translate(request, true, |_| {
            calls += 1;
            std::future::ready(Err(anyhow::anyhow!("model failed")))
        })
        .await;
        assert!(result.is_err());
        assert_eq!(calls, 1);
    }
}
