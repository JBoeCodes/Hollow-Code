//! Bundled, person-selected artwork. The default is represented by no override.
use crate::custom_icons::{IconRecord, Original};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::collections::BTreeMap;

macro_rules! artwork {
    ($name:literal) => {
        [
            (
                16,
                include_bytes!(concat!("../../public/app-icons/", $name, "/16.png")).as_slice(),
            ),
            (
                32,
                include_bytes!(concat!("../../public/app-icons/", $name, "/32.png")).as_slice(),
            ),
            (
                64,
                include_bytes!(concat!("../../public/app-icons/", $name, "/64.png")).as_slice(),
            ),
            (
                128,
                include_bytes!(concat!("../../public/app-icons/", $name, "/128.png")).as_slice(),
            ),
            (
                512,
                include_bytes!(concat!("../../public/app-icons/", $name, "/512.png")).as_slice(),
            ),
        ]
    };
}

pub fn record(id: &str) -> Result<Option<IconRecord>, String> {
    let images = match id {
        "fold" => return Ok(None),
        "cut-glass" => artwork!("cut-glass"),
        "threshold" => artwork!("threshold"),
        "aurora" => artwork!("aurora"),
        "monolith" => artwork!("monolith"),
        _ => return Err("Unknown app icon preset".into()),
    };
    // Preserve each authored size verbatim, especially the tiny representations.
    let pngs: BTreeMap<_, _> = images
        .into_iter()
        .map(|(size, png)| (size, STANDARD.encode(png)))
        .collect();
    let original = Original {
        format: "png".into(),
        bytes: pngs[&512].clone(),
    };
    let editor = serde_json::json!({"mode":"upload","fit":"fit"});
    use sha2::{Digest, Sha256};
    let revision = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(id, &original, &editor, &pngs)).map_err(|e| e.to_string())?
        )
    );
    Ok(Some(IconRecord {
        version: 1,
        revision,
        preset: Some(id.into()),
        original,
        editor,
        pngs,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::custom_icons::{make_record, read_at, write_at, SIZES};

    #[test]
    fn default_clears_override_and_unknown_names_are_refused() {
        assert!(record("fold").unwrap().is_none());
        assert!(record("../threshold").is_err());
        assert!(record("Custom").is_err());
    }

    #[test]
    fn presets_persist_exact_sizes_and_editing_becomes_custom() {
        for id in ["cut-glass", "threshold", "aurora", "monolith"] {
            let saved = record(id).unwrap().unwrap();
            assert_eq!(saved.pngs.keys().copied().collect::<Vec<_>>(), SIZES);
            for (size, encoded) in &saved.pngs {
                let data = STANDARD.decode(encoded).unwrap();
                let image = image::load_from_memory(&data).unwrap();
                assert_eq!((image.width(), image.height()), (*size, *size));
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("../public/app-icons/{id}/{size}.png"));
                assert_eq!(data, std::fs::read(path).unwrap());
            }
            let folder = tempfile::tempdir().unwrap();
            let path = folder.path().join("app.json");
            write_at(&path, Some(&saved)).unwrap();
            let restored = read_at(&path).unwrap().unwrap();
            assert_eq!(restored.preset.as_deref(), Some(id));
            assert_eq!(restored.pngs, saved.pngs);
            let edited = make_record(saved.original, &saved.pngs[&512], saved.editor).unwrap();
            assert!(edited.preset.is_none());
            write_at(&path, None).unwrap();
            assert!(read_at(&path).unwrap().is_none());
        }
    }

    #[test]
    fn pre_preset_records_stay_custom_without_migration() {
        let saved = record("threshold").unwrap().unwrap();
        let mut old = serde_json::to_value(saved).unwrap();
        old.as_object_mut().unwrap().remove("preset");
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("app.json");
        let bytes = serde_json::to_vec(&old).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert!(read_at(&path).unwrap().unwrap().preset.is_none());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}
