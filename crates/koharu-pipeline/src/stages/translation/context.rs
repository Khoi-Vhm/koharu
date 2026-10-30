use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use koharu_scene::{AssetRole, Component, EntityId, Origin, Snapshot, SourceText, Translation};
use koharu_translator::{Language, TranslationContext};
use revision::revisioned;

use crate::TranslationConfig;

#[revisioned(revision = 1)]
#[derive(Clone, Debug)]
pub(super) struct DialogueMemory {
    pub origin: Origin,
    pub fingerprint: String,
    pub summary: String,
}

impl Component for DialogueMemory {
    const KIND: &'static str = "dev.koharu.pipeline.dialogue-memory";

    fn origin(&self) -> Option<&Origin> {
        Some(&self.origin)
    }

    fn set_origin(&mut self, origin: Origin) -> bool {
        self.origin = origin;
        true
    }
}

pub(super) struct Context {
    pub entries: Vec<TranslationContext>,
    pub dialogue: Option<String>,
    pub hasher: blake3::Hasher,
}

pub(super) fn read(
    scene: &Snapshot,
    current: EntityId,
    config: &TranslationConfig,
) -> Result<Context> {
    let mut result = Context {
        entries: Vec::new(),
        dialogue: None,
        hasher: blake3::Hasher::new(),
    };
    result.hasher.update(&serde_json::to_vec(config)?);
    for page in scene.pages() {
        if page.id() == current {
            break;
        }
        let entries = append_page(scene, page.id(), &mut result.hasher, &BTreeMap::new())?;
        result
            .entries
            .extend(entries.into_iter().map(|(_, source, translation)| {
                TranslationContext {
                    source,
                    translation: translation
                        .filter(|translation| {
                            translation.language.as_ref().is_some_and(|language| {
                                language.as_str().parse::<Language>().ok()
                                    == Some(config.target_language)
                            })
                        })
                        .map_or_else(String::new, |translation| translation.text.value),
                }
            }));
        if let Some(memory) = scene.component::<DialogueMemory>(page.id())?
            && memory.fingerprint == result.hasher.finalize().to_hex().as_str()
        {
            result
                .hasher
                .update(&serde_json::to_vec(&("dialogue", &memory.summary))?);
            result.dialogue = Some(memory.summary);
        }
    }
    // Keep nearby dialogue bounded; the validated cumulative summary carries older relationships.
    let mut characters = 0;
    let keep = result
        .entries
        .iter()
        .rev()
        .take(64)
        .take_while(|entry| {
            characters += entry.source.chars().count() + entry.translation.chars().count();
            characters <= 16_000
        })
        .count();
    result.entries.drain(..result.entries.len() - keep);
    Ok(result)
}

pub(super) fn fingerprint(
    scene: &Snapshot,
    current: EntityId,
    mut prefix: blake3::Hasher,
    translated: &BTreeMap<EntityId, Translation>,
) -> Result<String> {
    append_page(scene, current, &mut prefix, translated)?;
    Ok(prefix.finalize().to_hex().to_string())
}

fn append_page(
    scene: &Snapshot,
    page: EntityId,
    hasher: &mut blake3::Hasher,
    translated: &BTreeMap<EntityId, Translation>,
) -> Result<Vec<(EntityId, String, Option<Translation>)>> {
    let asset = scene.asset(page, &AssetRole::new("source")?)?;
    let mut order = Vec::new();
    if let Some(group) = scene.page(page)?.text_group()? {
        for layer in group.text_layers()? {
            order.push((layer.id(), layer.content()?.id()));
        }
    }
    hasher.update(&serde_json::to_vec(&(
        page,
        asset.map(|asset| asset.blob),
        order,
    ))?);
    let mut content_ids = Vec::new();
    if let Some(group) = scene.page(page)?.text_group()? {
        for layer in group.text_layers()? {
            content_ids.push(layer.content()?.id());
        }
    }
    // Source-only documents have no presentation layers yet. Include their
    // remaining content after presented dialogue, without duplicating shared content.
    content_ids.extend(scene.subtree(page)?.map(|entity| entity.id()));
    let mut seen = BTreeSet::new();
    let mut entries = Vec::new();
    for id in content_ids {
        if !seen.insert(id) {
            continue;
        }
        let Some(source) = scene.component::<SourceText>(id)? else {
            continue;
        };
        let translation = translated
            .get(&id)
            .cloned()
            .or(scene.component::<Translation>(id)?);
        let region = scene
            .text_content(id)?
            .source_region()?
            .map(|region| region.geometry())
            .transpose()?;
        hasher.update(&serde_json::to_vec(&(
            id,
            &source.text.value,
            translation
                .as_ref()
                .map(|value| (&value.text.value, &value.language)),
            region.map(|geometry| geometry.points),
        ))?);
        if !source.text.value.trim().is_empty() {
            entries.push((id, source.text.value, translation));
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use koharu_scene::{At, Authored, LanguageTag, PageDraft, Session, SourceText, Translation};

    fn validate_translation_input(
        before: &Snapshot,
        after: &Snapshot,
        page: EntityId,
    ) -> Result<()> {
        let input = crate::stages::StageInput::new(
            before.clone(),
            page,
            None,
            None,
            std::sync::Arc::new(crate::ImageCache::default()),
            None,
        );
        super::super::TranslationInput::new(&input, &TranslationConfig::default())
            .validate_on(after)
    }

    #[tokio::test]
    async fn validation_ignores_generated_translation_that_is_not_prompt_context() {
        let mut session = Session::memory().await.unwrap();
        let mut edit = session.snapshot().edit();
        let page = edit
            .add_page(PageDraft::new("page", 100.0, 100.0), At::End)
            .unwrap();
        let content = edit.add_text_content(page, At::End).unwrap();
        edit.set(
            content,
            &SourceText {
                text: Authored::user("Hello".to_owned()),
                language: None,
            },
        )
        .unwrap();
        edit.add_text_layer(
            page,
            At::End,
            content,
            &koharu_scene::TextLayout {
                origin: Origin::User,
                kind: koharu_scene::TextLayoutKind::Paragraph,
                angle_degrees: None,
            },
        )
        .unwrap();
        session.commit(edit.finish().unwrap()).await.unwrap();
        let before = session.snapshot();
        let generated = koharu_scene::Generation::new(
            koharu_scene::ProducerId::new("test.translation").unwrap(),
        );
        let mut edit = before.edit_as(generated.clone());
        edit.set(
            content,
            &Translation {
                text: Authored::generated("Xin chào".to_owned(), generated),
                language: Some(LanguageTag::new("vi-VN").unwrap()),
            },
        )
        .unwrap();
        let patch = edit.finish().unwrap();
        let after = before.preview([&patch]).unwrap();
        validate_translation_input(&before, &after, page).unwrap();
    }

    #[tokio::test]
    async fn restores_memory_and_invalidates_it_after_source_or_translation_changes() {
        let mut session = Session::memory().await.unwrap();
        let mut edit = session.snapshot().edit();
        let first = edit
            .add_page(PageDraft::new("first", 100.0, 100.0), At::End)
            .unwrap();
        let second = edit
            .add_page(PageDraft::new("second", 100.0, 100.0), At::End)
            .unwrap();
        let third = edit
            .add_page(PageDraft::new("third", 100.0, 100.0), At::End)
            .unwrap();
        let content = edit.add_text_content(first, At::End).unwrap();
        edit.set(
            content,
            &SourceText {
                text: Authored::user("Master!".to_owned()),
                language: None,
            },
        )
        .unwrap();
        edit.set(
            content,
            &Translation {
                text: Authored::user("Thưa ngài!".to_owned()),
                language: Some(LanguageTag::new("vi").unwrap()),
            },
        )
        .unwrap();
        session.commit(edit.finish().unwrap()).await.unwrap();
        let config = crate::TranslationConfig {
            target_language: koharu_translator::Language::Vietnamese,
            ..Default::default()
        };
        let state = read(&session.snapshot(), first, &config).unwrap();
        let fingerprint =
            fingerprint(&session.snapshot(), first, state.hasher, &BTreeMap::new()).unwrap();
        let mut edit = session.snapshot().edit();
        edit.set(
            first,
            &DialogueMemory {
                origin: koharu_scene::Origin::User,
                fingerprint,
                summary: "A serves B.".to_owned(),
            },
        )
        .unwrap();
        session.commit(edit.finish().unwrap()).await.unwrap();
        let restored = read(&session.snapshot(), second, &config).unwrap();
        assert_eq!(restored.dialogue.as_deref(), Some("A serves B."));
        assert_eq!(restored.entries[0].translation, "Thưa ngài!");
        let inherited_fingerprint = super::fingerprint(
            &session.snapshot(),
            second,
            restored.hasher,
            &BTreeMap::new(),
        )
        .unwrap();
        let mut edit = session.snapshot().edit();
        edit.set(
            second,
            &DialogueMemory {
                origin: Origin::User,
                fingerprint: inherited_fingerprint,
                summary: "Old cumulative relationships".to_owned(),
            },
        )
        .unwrap();
        session.commit(edit.finish().unwrap()).await.unwrap();
        assert_eq!(
            read(&session.snapshot(), third, &config)
                .unwrap()
                .dialogue
                .as_deref(),
            Some("Old cumulative relationships")
        );
        let mut corrected = session
            .snapshot()
            .component::<DialogueMemory>(first)
            .unwrap()
            .unwrap();
        corrected.summary = "A and B are siblings".to_owned();
        let correction = session
            .snapshot()
            .patch(|edit| edit.set(first, &corrected))
            .unwrap();
        assert_eq!(
            read(
                &session.snapshot().preview([&correction]).unwrap(),
                third,
                &config
            )
            .unwrap()
            .dialogue
            .as_deref(),
            Some("A and B are siblings")
        );
        assert!(
            read(&session.snapshot(), first, &config)
                .unwrap()
                .dialogue
                .is_none()
        );
        let changed = session
            .snapshot()
            .patch(|edit| edit.move_entity(second, None, At::Start))
            .unwrap();
        assert!(
            read(
                &session.snapshot().preview([&changed]).unwrap(),
                third,
                &config
            )
            .unwrap()
            .dialogue
            .is_none()
        );
        let changed = session
            .snapshot()
            .patch(|edit| {
                edit.set_asset(
                    first,
                    &AssetRole::new("source").unwrap(),
                    koharu_scene::AssetInput::new(
                        &b"different page pixels"[..],
                        "image/png",
                        koharu_scene::AssetMetadata {
                            width: Some(1),
                            height: Some(1),
                            attributes: BTreeMap::new(),
                        },
                    ),
                )
            })
            .unwrap();
        assert!(
            read(
                &session.snapshot().preview([&changed]).unwrap(),
                second,
                &config
            )
            .unwrap()
            .dialogue
            .is_none()
        );
        let changed = session
            .snapshot()
            .patch(|edit| {
                edit.set(
                    content,
                    &SourceText {
                        text: Authored::user("Brother!".to_owned()),
                        language: None,
                    },
                )
            })
            .unwrap();
        assert!(
            read(
                &session.snapshot().preview([&changed]).unwrap(),
                second,
                &config
            )
            .unwrap()
            .dialogue
            .is_none()
        );
        let changed = session
            .snapshot()
            .patch(|edit| {
                edit.set(
                    content,
                    &Translation {
                        text: Authored::user("Anh ơi!".to_owned()),
                        language: Some(LanguageTag::new("vi").unwrap()),
                    },
                )
            })
            .unwrap();
        assert!(
            read(
                &session.snapshot().preview([&changed]).unwrap(),
                second,
                &config
            )
            .unwrap()
            .dialogue
            .is_none()
        );
        let mut different = config.clone();
        different.instructions = Some("Use archaic speech".to_owned());
        assert!(
            read(&session.snapshot(), second, &different)
                .unwrap()
                .dialogue
                .is_none()
        );
    }

    #[tokio::test]
    async fn context_follows_presentation_order_and_rejects_concurrent_reordering() {
        let mut session = Session::memory().await.unwrap();
        let mut edit = session.snapshot().edit();
        let page = edit
            .add_page(PageDraft::new("page", 100.0, 100.0), At::End)
            .unwrap();
        let next = edit
            .add_page(PageDraft::new("next", 100.0, 100.0), At::End)
            .unwrap();
        let mut layers = Vec::new();
        for source in ["first", "second"] {
            let content = edit.add_text_content(page, At::End).unwrap();
            edit.set(
                content,
                &SourceText {
                    text: Authored::user(source.to_owned()),
                    language: None,
                },
            )
            .unwrap();
            layers.push(
                edit.add_text_layer(
                    page,
                    At::End,
                    content,
                    &koharu_scene::TextLayout {
                        origin: Origin::User,
                        kind: koharu_scene::TextLayoutKind::Paragraph,
                        angle_degrees: None,
                    },
                )
                .unwrap(),
            );
        }
        session.commit(edit.finish().unwrap()).await.unwrap();
        let before = session.snapshot();
        let group = before
            .page(page)
            .unwrap()
            .text_group()
            .unwrap()
            .unwrap()
            .id();
        let patch = before
            .patch(|edit| edit.move_entity(layers[1], Some(group), At::Start))
            .unwrap();
        let after = before.preview([&patch]).unwrap();
        let context = read(&after, next, &TranslationConfig::default()).unwrap();
        assert_eq!(
            context
                .entries
                .iter()
                .map(|entry| entry.source.as_str())
                .collect::<Vec<_>>(),
            ["second", "first"]
        );
        assert!(validate_translation_input(&before, &after, next).is_err());
        let moved = before
            .patch(|edit| {
                edit.set(
                    layers[0],
                    &koharu_scene::Geometry::rectangle(50.0, 50.0, 10.0, 10.0),
                )
            })
            .unwrap();
        let after = before.preview([&moved]).unwrap();
        validate_translation_input(&before, &after, page).unwrap();
        let input = crate::stages::StageInput::new(
            before.clone(),
            page,
            None,
            Some(crate::Bounds {
                x: 40.0,
                y: 40.0,
                width: 40.0,
                height: 40.0,
            }),
            std::sync::Arc::new(crate::ImageCache::default()),
            None,
        );
        assert!(
            super::super::TranslationInput::new(&input, &TranslationConfig::default())
                .validate_on(&after)
                .is_err()
        );
    }
}
