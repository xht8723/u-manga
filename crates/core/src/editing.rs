//! Apply only deliberate Editor changes to the latest persisted page.
use crate::types::*;
use anyhow::{Result, ensure};

/// A moved text box cannot keep using a detector association at its former location.
pub fn validate_balloon(region: &mut Region) {
    if region.bubble.is_some_and(|b| {
        b[0] > region.bbox[0]
            || b[1] > region.bbox[1]
            || b[2] < region.bbox[2]
            || b[3] < region.bbox[3]
    }) {
        region.bubble = None;
    }
}

pub fn merge(base: &Page, draft: &Page, latest: &Page) -> Result<Page> {
    ensure!(
        base.id == draft.id && draft.id == latest.id,
        "Editor page changed"
    );
    let mut merged = latest.clone();
    if base.background != draft.background {
        merged.background = draft.background.clone();
    }
    for old in &base.regions {
        if !draft.regions.iter().any(|r| r.id == old.id) {
            merged.regions.retain(|r| r.id != old.id);
        }
    }
    for (index, edited) in draft.regions.iter().enumerate() {
        let Some(old) = base.regions.iter().find(|r| r.id == edited.id) else {
            ensure!(
                !merged.regions.iter().any(|r| r.id == edited.id),
                "Region ID already exists"
            );
            let mut added = edited.clone();
            added.prepared = false; // Only an explicitly submitted preparation job grants this.
            // Preserve the restored region's place without moving newer pipeline regions.
            let insertion = draft.regions[index + 1..]
                .iter()
                .find_map(|next| merged.regions.iter().position(|r| r.id == next.id))
                .unwrap_or(merged.regions.len());
            merged.regions.insert(insertion, added);
            continue;
        };
        let old_json = serde_json::to_value(old)?;
        let edited_json = serde_json::to_value(edited)?;
        if old_json == edited_json {
            continue;
        }
        let current = merged
            .regions
            .iter_mut()
            .find(|r| r.id == edited.id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "An edited region was removed; discard this draft and select a current region"
                )
            })?;
        let mut json = serde_json::to_value(&*current)?;
        // Detection identity/classification and review messages belong to processing.
        for key in [
            "bbox",
            "source",
            "target",
            "direction",
            "allowFill",
            "overlayOnly",
        ] {
            if old_json[key] != edited_json[key] {
                json[key] = edited_json[key].clone();
            }
        }
        for key in [
            "font",
            "size",
            "color",
            "fill",
            "lineGap",
            "outlineEnabled",
            "outlineWidthPercent",
            "outlineColor",
        ] {
            if old_json["style"][key] != edited_json["style"][key] {
                json["style"][key] = edited_json["style"][key].clone();
            }
        }
        *current = serde_json::from_value(json)?;
        if old.bbox != edited.bbox {
            validate_balloon(current);
        }
    }
    Ok(merged)
}
