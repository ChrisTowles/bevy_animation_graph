use bevy::ecs::{component::Component, reflect::ReflectComponent};
use bevy::reflect::{Reflect, prelude::ReflectDefault};
use bevy_animation_graph_core::{
    animation_clip::EntityPath,
    animation_node::{NodeLike, ReflectNodeLike},
    context::{new_context::NodeContext, spec_context::SpecContext},
    edge_data::{DataSpec, DataValue},
    errors::GraphError,
};

/// A fixed bone path, for nodes that target a bone (`RotationNode`, `TwoBoneIKNode`): the
/// bone names from the skeleton root down. Plain strings, so a `.bsn` graph spells them as a
/// list of literals.
#[derive(Component, Reflect, Clone, Debug, Default)]
#[reflect(Component, Default, NodeLike)]
#[type_path = "bevy_animation_graph::builtin_nodes"]
pub struct ConstEntityPath {
    pub path: Vec<String>,
}

impl ConstEntityPath {
    pub const OUTPUT: &'static str = "out";

    pub fn new(path: EntityPath) -> Self {
        Self {
            path: path.into(),
        }
    }
}

impl NodeLike for ConstEntityPath {
    fn display_name(&self) -> String {
        "Entity Path".into()
    }

    fn update(&self, mut ctx: NodeContext) -> Result<(), GraphError> {
        ctx.set_data_fwd(
            Self::OUTPUT,
            DataValue::EntityPath(EntityPath::from(self.path.clone())),
        );
        Ok(())
    }

    fn spec(&self, mut ctx: SpecContext) -> Result<(), GraphError> {
        ctx.add_output_data(Self::OUTPUT, DataSpec::EntityPath);

        Ok(())
    }
}
