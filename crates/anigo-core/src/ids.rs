//! ANIGO Canonical Stable Identifiers (P0 — ARQUITETURA_CANONICA_ANIGO §3.1).
//!
//! Every persistent entity in `ProjectState` is referenced by a stable,
//! string-encoded identifier — never by array position. The wire format is
//! part of the versioned contract shared with TypeScript
//! (`contracts/anigo.project.v1.schema.json`), so the rules below are frozen:
//!
//! ```text
//! <prefix>_<slug>
//!   prefix : one of the canonical prefixes declared in this module
//!   slug   : [a-z0-9_]+ (1..=ID_MAX_LEN - prefix_len - 1 bytes)
//! ```
//!
//! Rules enforced by `parse`:
//! - the prefix must match the entity kind (no `mat_` id can be used as a node);
//! - the slug charset is lowercase ASCII alphanumeric plus `_`;
//! - the total length never exceeds `ID_MAX_LEN` (128 bytes).
//!
//! Ids are *derived*, never random: `MorphId::for_slider("head_width")` always
//! yields `mrf_head_width`, so a saved project keeps pointing at the same
//! entity across sessions, machines and schema migrations.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Maximum accepted length of an encoded id (prefix + `_` + slug), in bytes.
pub const ID_MAX_LEN: usize = 128;

/// Canonical prefixes. These strings are contract-stable: changing one is a
/// breaking schema change and requires a migration.
pub const PREFIX_PROJECT: &str = "prj";
pub const PREFIX_CHARACTER: &str = "chr";
pub const PREFIX_MORPH: &str = "mrf";
pub const PREFIX_SCENE: &str = "scn";
pub const PREFIX_NODE: &str = "nod";
pub const PREFIX_MATERIAL: &str = "mat";
pub const PREFIX_ASSET: &str = "ast";
pub const PREFIX_CAMERA: &str = "cam";
pub const PREFIX_LIGHT: &str = "lgt";
pub const PREFIX_ANIMATION: &str = "ani";

/// Failures produced while parsing or deriving an identifier.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    /// The id was empty or contained only whitespace.
    #[error("empty identifier for {expected}")]
    Empty { expected: &'static str },
    /// The id exceeded `ID_MAX_LEN`.
    #[error("identifier '{found}' is too long: {} bytes (max {ID_MAX_LEN})", found.len())]
    TooLong { found: String },
    /// The id did not contain the `<prefix>_<slug>` separator.
    #[error("identifier '{found}' is missing the '<prefix>_<slug>' separator (expected prefix '{expected}')")]
    MissingSeparator {
        expected: &'static str,
        found: String,
    },
    /// The prefix did not match the entity kind.
    #[error("identifier '{found}' has prefix '{actual}', expected '{expected}'")]
    WrongPrefix {
        expected: &'static str,
        actual: String,
        found: String,
    },
    /// The slug was empty or contained illegal characters.
    #[error("identifier '{found}' has an invalid slug: must match [a-z0-9_]+ and be non-empty")]
    InvalidSlug { found: String },
}

/// Shared behaviour of every stable identifier.
pub trait StableId: Sized + Clone + Eq + Ord + fmt::Debug + fmt::Display {
    /// Canonical prefix of this entity kind (`chr`, `mrf`, …).
    const PREFIX: &'static str;
    /// Human-readable entity kind used in error messages.
    const LABEL: &'static str;

    /// Borrows the encoded form (`<prefix>_<slug>`).
    fn as_str(&self) -> &str;

    /// Validates and constructs an identifier from its encoded form.
    fn parse(raw: impl AsRef<str>) -> Result<Self, IdError>;

    /// Builds an identifier from an arbitrary label, deterministically.
    ///
    /// The label is slugified; when the result would be empty a stable hash of
    /// the original label is used instead, so the function is total.
    fn from_label(label: &str) -> Self {
        Self::from_slug(&slugify(label))
    }

    /// Builds an identifier from an already-valid slug (or slugifies it).
    ///
    /// Never fails: the produced value always satisfies `parse`.
    fn from_slug(slug: &str) -> Self {
        let clean = if is_valid_slug(slug) {
            slug.to_string()
        } else {
            let slugified = slugify(slug);
            if is_valid_slug(&slugified) {
                slugified
            } else {
                format!("{:016x}", fnv1a64(slug))
            }
        };
        // Truncation keeps the total length inside ID_MAX_LEN.
        let budget = ID_MAX_LEN.saturating_sub(Self::PREFIX.len() + 1);
        let truncated: String = clean.chars().take(budget).collect();
        let mut encoded = String::with_capacity(Self::PREFIX.len() + 1 + truncated.len());
        encoded.push_str(Self::PREFIX);
        encoded.push('_');
        encoded.push_str(&truncated);
        // `from_slug` is infallible by construction: see `slug_is_always_valid`.
        Self::from_encoded_unchecked(encoded)
    }

    /// Rebuilds an identifier from a string that is already known to satisfy
    /// `parse`. Implementations must not validate again.
    #[doc(hidden)]
    fn from_encoded_unchecked(encoded: String) -> Self;
}

/// Returns `true` when `slug` matches `[a-z0-9_]+`.
pub fn is_valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= ID_MAX_LEN
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Deterministic, ASCII-only slugification (`"Cabeça / Head" -> "cabeca_head"`).
pub fn slugify(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut pending_sep = false;
    for ch in label.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_sep && !out.is_empty() {
                out.push('_');
            }
            pending_sep = false;
            out.push(ch.to_ascii_lowercase());
        } else if ch.is_whitespace() || ch == '-' || ch == '_' || ch == '/' || ch == '.' {
            pending_sep = true;
        } else {
            // Non-ASCII or symbol: fold to an ASCII-safe separator so ids stay
            // portable across filesystems/URLs (accents are handled below).
            let folded = fold_ascii(ch);
            match folded {
                Some(c) => {
                    if pending_sep && !out.is_empty() {
                        out.push('_');
                    }
                    pending_sep = false;
                    out.push(c);
                }
                None => pending_sep = true,
            }
        }
    }
    out
}

/// Minimal accent folding for the Portuguese/Japanese-friendly labels used by
/// the ANIGO catalog (no external crates, deterministic).
fn fold_ascii(ch: char) -> Option<char> {
    let lower = ch.to_ascii_lowercase();
    let mapped = match lower {
        'á' | 'à' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        _ => return None,
    };
    Some(mapped)
}

/// 64-bit FNV-1a — the same function used by the TypeScript reference
/// (`src/services/morph_engine.ts::hashString32` is its 32-bit sibling), so
/// derived ids are reproducible on both sides of the contract.
pub fn fnv1a64(input: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in input.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Declares a stable identifier newtype with validation + serde support.
macro_rules! define_stable_id {
    ($(#[$meta:meta])* $name:ident, $prefix:expr, $label:expr) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl StableId for $name {
            const PREFIX: &'static str = $prefix;
            const LABEL: &'static str = $label;

            fn as_str(&self) -> &str {
                &self.0
            }

            fn parse(raw: impl AsRef<str>) -> Result<Self, IdError> {
                let raw = raw.as_ref();

                if raw.trim().is_empty() {
                    return Err(IdError::Empty { expected: $label });
                }
                if raw.len() > ID_MAX_LEN {
                    return Err(IdError::TooLong { found: raw.to_string() });
                }
                let (prefix, slug) = raw
                    .split_once('_')
                    .ok_or_else(|| IdError::MissingSeparator {
                        expected: $prefix,
                        found: raw.to_string(),
                    })?;
                if prefix != $prefix {
                    return Err(IdError::WrongPrefix {
                        expected: $prefix,
                        actual: prefix.to_string(),
                        found: raw.to_string(),
                    });
                }
                if !is_valid_slug(slug) {
                    return Err(IdError::InvalidSlug { found: raw.to_string() });
                }

                Ok(Self(raw.to_string()))
            }

            fn from_encoded_unchecked(encoded: String) -> Self {
                Self(encoded)
            }
        }

        impl $name {
            /// Canonical prefix (`chr`, `mrf`, …).
            pub const PREFIX: &'static str = $prefix;
            /// Human-readable entity kind.
            pub const LABEL: &'static str = $label;

            /// Validates and constructs the identifier.
            pub fn parse(raw: impl AsRef<str>) -> Result<Self, IdError> {
                <Self as StableId>::parse(raw)
            }

            /// Adds the canonical prefix to `slug`, slugifying when needed.
            pub fn from_slug(slug: &str) -> Self {
                <Self as StableId>::from_slug(slug)
            }

            /// Derives the identifier from a human label (deterministic).
            pub fn from_label(label: &str) -> Self {
                <Self as StableId>::from_label(label)
            }

            /// Borrows the encoded form.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Consumes the identifier, returning the encoded form.
            pub fn into_string(self) -> String {
                self.0
            }

            /// Prefix guarantee check used by contract tests.
            pub fn has_canonical_prefix(&self) -> bool {
                self.0
                    .strip_prefix(Self::PREFIX)
                    .is_some_and(|rest| rest.starts_with('_'))
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = IdError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                <Self as StableId>::parse(s)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = IdError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                <Self as StableId>::parse(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                <Self as StableId>::parse(value)
            }
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let raw = String::deserialize(deserializer)?;
                <Self as StableId>::parse(raw).map_err(serde::de::Error::custom)
            }
        }
    };
}

define_stable_id!(
    /// Stable identifier of a saved project (`prj_<slug>`).
    ProjectId,
    PREFIX_PROJECT,
    "project"
);
define_stable_id!(
    /// Stable identifier of the character authored in the project.
    CharacterId,
    PREFIX_CHARACTER,
    "character"
);
define_stable_id!(
    /// Stable identifier of a canonical morph slider target.
    MorphId,
    PREFIX_MORPH,
    "morph"
);
define_stable_id!(
    /// Stable identifier of a scene.
    SceneId,
    PREFIX_SCENE,
    "scene"
);
define_stable_id!(
    /// Stable identifier of a scene node.
    NodeId,
    PREFIX_NODE,
    "scene node"
);
define_stable_id!(
    /// Stable identifier of a material.
    MaterialId,
    PREFIX_MATERIAL,
    "material"
);
define_stable_id!(
    /// Stable identifier of an imported/referenced asset.
    AssetId,
    PREFIX_ASSET,
    "asset"
);
define_stable_id!(
    /// Stable identifier of a project camera.
    CameraId,
    PREFIX_CAMERA,
    "camera"
);
define_stable_id!(
    /// Stable identifier of a light.
    LightId,
    PREFIX_LIGHT,
    "light"
);
define_stable_id!(
    /// Stable identifier of an animation clip.
    AnimationClipId,
    PREFIX_ANIMATION,
    "animation clip"
);

impl MorphId {
    /// Canonical morph id of a catalog slider: `head_width -> mrf_head_width`.
    ///
    /// The mapping is 1:1 and frozen by the catalog contract, so a saved
    /// weight always resolves to the same slider.
    pub fn for_slider(slider_id: &str) -> Self {
        Self::from_slug(slider_id)
    }

    /// Recovers the catalog slider id (`mrf_head_width -> head_width`).
    pub fn slider_id(&self) -> &str {
        self.0
            .strip_prefix("mrf_")
            .unwrap_or(self.0.as_str())
    }
}

impl AssetId {
    /// Content-addressed asset id: the same URI always maps to the same id.
    pub fn for_uri(uri: &str) -> Self {
        let digest = fnv1a64(&normalize_uri(uri));
        Self::from_slug(&format!("{:016x}", digest))
    }
}

/// Canonical URI normalization used for asset id derivation (case-stable,
/// separator-stable, no query strings).
pub fn normalize_uri(uri: &str) -> String {
    let mut normalized = uri.trim().replace('\\', "/");
    if let Some(idx) = normalized.find('?') {
        normalized.truncate(idx);
    }
    while normalized.starts_with("./") {
        normalized.drain(..2);
    }
    normalized.to_ascii_lowercase()
}

impl CharacterId {
    /// Canonical character slot of a freshly created project.
    pub fn canonical() -> Self {
        Self::from_slug("canonical")
    }
}

impl SceneId {
    /// Canonical scene slot of a freshly created project.
    pub fn canonical() -> Self {
        Self::from_slug("main")
    }
}

impl CameraId {
    /// Canonical viewport camera slot.
    pub fn canonical() -> Self {
        Self::from_slug("viewport")
    }
}

impl LightId {
    /// Canonical key light slot.
    pub fn canonical_key() -> Self {
        Self::from_slug("key")
    }
}

impl MaterialId {
    /// Canonical default anime material slot.
    pub fn canonical_default() -> Self {
        Self::from_slug("default_anime")
    }
}

impl NodeId {
    /// Canonical node holding the character mesh.
    pub fn canonical_character() -> Self {
        Self::from_slug("character_base")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_are_unique_and_stable() {
        let prefixes = [
            PREFIX_PROJECT,
            PREFIX_CHARACTER,
            PREFIX_MORPH,
            PREFIX_SCENE,
            PREFIX_NODE,
            PREFIX_MATERIAL,
            PREFIX_ASSET,
            PREFIX_CAMERA,
            PREFIX_LIGHT,
            PREFIX_ANIMATION,
        ];
        let unique: std::collections::HashSet<&str> = prefixes.iter().copied().collect();
        assert_eq!(unique.len(), prefixes.len(), "prefixes must be unique");
        for prefix in prefixes {
            assert!(is_valid_slug(prefix), "prefix '{prefix}' must be slug-safe");
        }
    }

    #[test]
    fn parse_accepts_canonical_ids_and_rejects_the_rest() {
        assert_eq!(MorphId::parse("mrf_head_width").unwrap().as_str(), "mrf_head_width");
        assert!(MorphId::parse("mat_head_width").is_err(), "wrong prefix must fail");
        assert!(MorphId::parse("mrf_").is_err(), "empty slug must fail");
        assert!(MorphId::parse("mrf_HeadWidth").is_err(), "uppercase slug must fail");
        assert!(MorphId::parse("mrf-head-width").is_err(), "dash separator must fail");
        assert!(MorphId::parse("head_width").is_err(), "missing prefix must fail");
        assert!(MorphId::parse("").is_err(), "empty id must fail");
        let long = format!("mrf_{}", "a".repeat(ID_MAX_LEN));
        assert!(MorphId::parse(long).is_err(), "length limit must be enforced");
    }

    #[test]
    fn derived_ids_are_deterministic_and_valid() {
        assert_eq!(MorphId::for_slider("head_width").as_str(), "mrf_head_width");
        assert_eq!(
            MorphId::for_slider("head_width"),
            MorphId::for_slider("head_width"),
            "derivation must be pure"
        );
        assert_eq!(MorphId::for_slider("head_width").slider_id(), "head_width");

        let from_label = CharacterId::from_label("Cabeça / Head");
        assert_eq!(from_label.as_str(), "chr_cabeca_head");
        assert!(from_label.has_canonical_prefix());
        CharacterId::parse(from_label.as_str()).expect("derived ids always parse");

        let unicode = NodeId::from_label("日本語モデル");
        assert!(is_valid_slug(unicode.as_str().trim_start_matches("nod_")));
        NodeId::parse(unicode.as_str()).expect("unicode labels must still yield valid ids");
    }

    #[test]
    fn slugify_folds_accents_and_separators() {
        assert_eq!(slugify("Altura Estelar Total"), "altura_estelar_total");
        assert_eq!(slugify("Gordura / Corpulência Macro"), "gordura_corpulencia_macro");
        assert_eq!(slugify("--__"), "");
        assert_eq!(slugify("v2.0 - final"), "v2_0_final");
    }

    #[test]
    fn asset_ids_are_content_addressed() {
        let a = AssetId::for_uri("assets/models/anigo_base_male.glb");
        let b = AssetId::for_uri("./ASSETS\\Models\\anigo_base_male.glb?x=1");
        assert_eq!(a, b, "URI normalization must be canonical");
        assert!(a.has_canonical_prefix());
        let c = AssetId::for_uri("assets/models/anigo_base_female.glb");
        assert_ne!(a, c);
    }

    #[test]
    fn serde_round_trip_preserves_ids_and_rejects_invalid_ones() {
        let id = MaterialId::from_slug("default_anime");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"mat_default_anime\"");
        let back: MaterialId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);

        let err = serde_json::from_str::<MaterialId>("\"nod_oops\"");
        assert!(err.is_err(), "deserializing a foreign prefix must fail");
    }

    #[test]
    fn map_keys_use_the_encoded_form() {
        let mut map: std::collections::BTreeMap<MaterialId, u32> =
            std::collections::BTreeMap::new();
        map.insert(MaterialId::from_slug("skin"), 1);
        map.insert(MaterialId::from_slug("cloth"), 2);
        let json = serde_json::to_string(&map).unwrap();
        assert_eq!(json, "{\"mat_cloth\":2,\"mat_skin\":1}");
        let back: std::collections::BTreeMap<MaterialId, u32> =
            serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), 2);
    }
}
