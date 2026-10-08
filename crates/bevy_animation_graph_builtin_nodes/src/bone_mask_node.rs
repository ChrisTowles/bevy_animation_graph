use bevy::asset::Handle;
use bevy::ecs::{component::Component, reflect::ReflectComponent};
use bevy::reflect::{Reflect, prelude::ReflectDefault};
use bevy_animation_graph_core::{
    animation_node::{NodeLike, ReflectNodeLike},
    bone_mask_asset::BoneMaskAsset,
    context::{new_context::NodeContext, spec_context::SpecContext},
    edge_data::{DataSpec, DataValue, bone_mask::BoneMask},
    errors::GraphError,
};

/// A `.mask.ron` bone mask, for a blend's `bone_mask` pin. Until the asset loads it masks
/// nothing in (an empty positive mask), so the blend shows its base pose.
#[derive(Component, Reflect, Clone, Debug, Default)]
#[reflect(Component, Default, NodeLike)]
#[type_path = "bevy_animation_graph::builtin_nodes"]
pub struct BoneMaskNode {
    pub mask: Handle<BoneMaskAsset>,
}

impl BoneMaskNode {
    pub const OUTPUT: &'static str = "mask";

    pub fn new(mask: Handle<BoneMaskAsset>) -> Self {
        Self { mask }
    }
}

impl NodeLike for BoneMaskNode {
    fn display_name(&self) -> String {
        "Bone Mask".into()
    }

    fn update(&self, mut ctx: NodeContext) -> Result<(), GraphError> {
        let mask = ctx
            .graph_context
            .resources
            .bone_mask_assets
            .get(&self.mask)
            .map_or_else(BoneMask::none, |asset| asset.mask.clone());
        ctx.set_data_fwd(Self::OUTPUT, DataValue::BoneMask(mask));
        Ok(())
    }

    fn spec(&self, mut ctx: SpecContext) -> Result<(), GraphError> {
        ctx.add_output_data(Self::OUTPUT, DataSpec::BoneMask);
        Ok(())
    }
}
