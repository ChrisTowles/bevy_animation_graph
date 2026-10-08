//! A bone mask as an asset (`.mask.ron`): which bones a layer touches, and how much.
//!
//! ```ron
//! (
//!     skeleton: "characters/human_male/human_male.skn.ron",
//!     base: Positive,
//!     weights: {
//!         ["humanmale", "humanmale_skeleton", ..., "bone_SpineLow"]: 1.0,
//!     },
//! )
//! ```
//!
//! Weights are per bone and do not inherit: an upper-body mask lists every bone under the spine.
//! `skeleton` names the rig the paths come from, so an editor can show the tree. A
//! `BoneMaskNode` feeds it to a blend's `bone_mask` pin.

use bevy::{
    asset::{Asset, AssetLoader, Handle, LoadContext, io::Reader},
    platform::collections::HashMap,
    reflect::{Reflect, TypePath},
};
use serde::{Deserialize, Serialize};

use crate::{
    animation_clip::EntityPath,
    edge_data::bone_mask::{BoneMask, BoneMaskSerial, BoneMaskType},
    errors::AssetLoaderError,
    skeleton::Skeleton,
};

#[derive(Asset, Reflect, Clone, Default, Debug)]
pub struct BoneMaskAsset {
    pub mask: BoneMask,
    pub skeleton: Handle<Skeleton>,
}

/// The `.mask.ron` file.
#[derive(Serialize, Deserialize)]
pub struct BoneMaskAssetSerial {
    pub skeleton: String,
    pub base: BoneMaskType,
    pub weights: HashMap<EntityPath, f32>,
}

impl BoneMaskAssetSerial {
    /// The file form of `mask`, its rig named by `skeleton`'s asset path. Weights are written in
    /// path order so a save diffs cleanly.
    pub fn from_mask(mask: &BoneMask, skeleton: String) -> Self {
        let serial = BoneMaskSerial::from_value(mask);
        Self {
            skeleton,
            base: serial.base,
            weights: serial.weights,
        }
    }

    pub fn to_ron(&self) -> Result<String, ron::Error> {
        let mut weights: Vec<(&EntityPath, &f32)> = self.weights.iter().collect();
        weights.sort_by(|a, b| a.0.cmp(b.0));
        let mut out = format!(
            "(\n    skeleton: {:?},\n    base: {:?},\n    weights: {{\n",
            self.skeleton, self.base
        );
        for (path, weight) in weights {
            let parts: Vec<String> = path.clone().into();
            out.push_str(&format!("        {}: {weight},\n", ron::to_string(&parts)?));
        }
        out.push_str("    },\n)\n");
        Ok(out)
    }
}

#[derive(Default, TypePath)]
pub struct BoneMaskLoader;

impl AssetLoader for BoneMaskLoader {
    type Asset = BoneMaskAsset;
    type Settings = ();
    type Error = AssetLoaderError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = vec![];
        reader.read_to_end(&mut bytes).await?;
        let serial: BoneMaskAssetSerial = ron::de::from_bytes(&bytes)?;
        let mask = BoneMaskSerial {
            weights: serial.weights,
            base: serial.base,
        }
        .to_value();
        Ok(BoneMaskAsset {
            mask,
            skeleton: load_context.load(serial.skeleton),
        })
    }

    fn extensions(&self) -> &[&str] {
        &["mask.ron"]
    }
}
