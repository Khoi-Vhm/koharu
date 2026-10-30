pub(super) mod context;

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use anyhow::{Result, ensure};
use async_trait::async_trait;
use koharu_scene::{
    AssetRole, Authored, EntityId, LanguageTag, Origin, Snapshot, SourceText, Translation,
};
use koharu_translator::{Language, TranslationContext, TranslationRequest, Translator};

use crate::{Bounds, TranslationConfig};

use super::{StageInput, StageProcessor, finish, generation};

const PRODUCER: &str = "dev.koharu.pipeline.translation";

pub(super) struct Processor {
    config: TranslationConfig,
    translator: Translator,
}

impl Processor {
    pub(super) fn validation(&self, input: &StageInput) -> TranslationInput {
        TranslationInput::new(input, &self.config)
    }

    pub(super) fn new(config: TranslationConfig, translator: Translator) -> Self {
        Self { config, translator }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TranslationInput {
    scene: Snapshot,
    page: EntityId,
    entities: Option<Arc<BTreeSet<EntityId>>>,
    region: Option<Bounds>,
    config: TranslationConfig,
}

impl TranslationInput {
    pub(super) fn new(input: &StageInput, config: &TranslationConfig) -> Self {
        Self {
            scene: input.scene.clone(),
            page: input.page,
            entities: input.entities.clone(),
            region: input.region,
            config: config.clone(),
        }
    }

    pub(crate) fn validate_on(&self, current: &Snapshot) -> Result<()> {
        let before = prepare(
            &self.scene,
            self.page,
            self.entities.as_deref(),
            self.region,
            &self.config,
        )?;
        let after = prepare(
            current,
            self.page,
            self.entities.as_deref(),
            self.region,
            &self.config,
        )?;
        ensure!(
            before.request == after.request && before.targets == after.targets,
            "dialogue context changed during translation; rerun translation for this page"
        );
        if Translator::supports_vision(&self.config.model, &self.config.generation) {
            let role = AssetRole::new("source")?;
            ensure!(
                self.scene.asset(self.page, &role)?.map(|asset| asset.blob)
                    == current.asset(self.page, &role)?.map(|asset| asset.blob),
                "source image changed during translation; rerun translation for this page"
            );
        }
        Ok(())
    }
}

struct Prepared {
    request: TranslationRequest,
    targets: Vec<(EntityId, String)>,
    prefix: blake3::Hasher,
}

fn prepare(
    scene: &Snapshot,
    page: EntityId,
    entities: Option<&BTreeSet<EntityId>>,
    region: Option<Bounds>,
    config: &TranslationConfig,
) -> Result<Prepared> {
    let mut targets = Vec::new();
    let mut regions = Vec::new();
    let mut page_context = Vec::new();
    if let Some(group) = scene.page(page)?.text_group()? {
        for layer in group.text_layers()? {
            let content = layer.content()?;
            let Some(source) = content.source()? else {
                continue;
            };
            if let Some(translation) = content.translation()?
                && matches!(translation.text.origin, Origin::User)
            {
                page_context.push(TranslationContext {
                    source: source.text.value.clone(),
                    translation: translation.text.value,
                });
            }
            if !crate::scope::contains_entity(scene, page, entities, region, layer.id())? {
                page_context.push(TranslationContext {
                    source: source.text.value,
                    translation: content
                        .translation()?
                        .filter(|translation| {
                            translation.language.as_ref().is_some_and(|language| {
                                language.as_str().parse::<Language>().ok()
                                    == Some(config.target_language)
                            })
                        })
                        .map_or_else(String::new, |translation| translation.text.value),
                });
                continue;
            }
            if !source.text.value.trim().is_empty() {
                regions.push(
                    content
                        .source_region()?
                        .map(|region| region.geometry())
                        .transpose()?
                        .map_or_else(Vec::new, |geometry| {
                            geometry
                                .points
                                .into_iter()
                                .map(|point| [point.x, point.y])
                                .collect()
                        }),
                );
                targets.push((content.id(), source.text.value));
            }
        }
    }
    let mut request = TranslationRequest::new(
        targets.iter().map(|(_, source)| source.clone()),
        config.target_language,
    );
    let context = context::read(scene, page, config)?;
    request.context = context.entries;
    request.context.extend(page_context);
    request.dialogue = context.dialogue;
    request.regions = regions;
    if let Some(instructions) = config.instructions.as_deref() {
        request = request.with_instructions(instructions);
    }
    Ok(Prepared {
        request,
        targets,
        prefix: context.hasher,
    })
}

#[async_trait]
impl StageProcessor for Processor {
    fn model(&self) -> &'static str {
        Translator::model(&self.config.model)
    }

    fn unload(&self) -> bool {
        self.translator.unload()
    }

    async fn load(&self) -> Result<()> {
        self.translator.load_model(&self.config.model).await
    }

    async fn process(&self, input: StageInput) -> Result<koharu_scene::Patch> {
        let Prepared {
            mut request,
            targets,
            prefix,
        } = prepare(
            &input.scene,
            input.page,
            input.entities.as_deref(),
            input.region,
            &self.config,
        )?;
        if Translator::supports_vision(&self.config.model, &self.config.generation)
            && let Some(image) = input.images.get(&input.scene, input.page, "source").await?
        {
            request = request.with_image(image);
        }
        let result = self
            .translator
            .translate(&self.config.model, self.config.generation, request)
            .await?;
        let language = LanguageTag::new(self.config.target_language.tag())?;
        let generated = generation(PRODUCER, result.provider)?;
        let mut edit = input.scene.edit_as(generated.clone());
        for (entity, _) in &targets {
            edit.observe::<SourceText>(*entity)?;
            edit.observe::<Translation>(*entity)?;
        }
        let mut translated = BTreeMap::new();
        for ((entity, source), text) in targets.into_iter().zip(result.translations) {
            if input
                .scene
                .component::<Translation>(entity)?
                .is_some_and(|value| matches!(value.text.origin, Origin::User))
            {
                continue;
            }
            let text = if source.trim() == "\u{2026}" {
                "\u{2026}".to_owned()
            } else {
                text
            };
            let translation = Translation {
                text: Authored::generated(text, generated.clone()),
                language: Some(language.clone()),
            };
            edit.set(entity, &translation)?;
            translated.insert(entity, translation);
        }
        // A scoped rerun must not replace a full-page memory with partial observations.
        if let Some(summary) = result.dialogue
            && input.entities.is_none()
            && input.region.is_none()
        {
            edit.observe::<context::DialogueMemory>(input.page)?;
            edit.observe_assets(input.page)?;
            edit.set(
                input.page,
                &context::DialogueMemory {
                    origin: Origin::Generated(generated),
                    fingerprint: context::fingerprint(
                        &input.scene,
                        input.page,
                        prefix,
                        &translated,
                    )?,
                    summary,
                },
            )?;
        }
        finish(edit)
    }
}
