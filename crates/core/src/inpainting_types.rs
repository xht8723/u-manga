use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupMethod {
    Solid,
    Migan,
    MangaAot,
    #[default]
    MangaLama,
}
impl CleanupMethod {
    pub fn id(self) -> &'static str {
        match self {
            Self::Solid => "solid",
            Self::Migan => "migan",
            Self::MangaAot => "manga_aot",
            Self::MangaLama => "manga_lama",
        }
    }
    pub fn pack(self) -> Option<&'static str> {
        match self {
            Self::Solid => None,
            Self::Migan => Some("inpaint_migan"),
            Self::MangaAot => Some("inpaint_manga_aot"),
            Self::MangaLama => Some("inpaint_manga_lama"),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Solid => "Solid fill",
            Self::Migan => "MI-GAN",
            Self::MangaAot => "Manga AOT",
            Self::MangaLama => "Manga LaMa",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupStrategy {
    #[default]
    Automatic,
    AllRegions,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupDevice {
    Auto,
    Cpu,
    #[default]
    Directml,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupSettings {
    pub method: CleanupMethod,
    pub strategy: CleanupStrategy,
    pub device: CleanupDevice,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageCleanup {
    pub settings: CleanupSettings,
    pub key: String,
    pub path: String,
    pub device: String,
    #[serde(with = "crate::ui_message::map")]
    pub reviews: BTreeMap<String, String>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    #[default]
    Translation,
    Cleanup,
    Preparation,
}
