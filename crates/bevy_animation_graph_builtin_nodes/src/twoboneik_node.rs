use bevy::ecs::{component::Component, reflect::ReflectComponent};
use bevy::{
    color::LinearRgba,
    math::{Quat, Vec3},
    reflect::{Reflect, std_traits::ReflectDefault},
};
use bevy_animation_graph_core::{
    animation_node::{NodeLike, ReflectNodeLike},
    context::{new_context::NodeContext, spec_context::SpecContext},
    edge_data::DataSpec,
    errors::GraphError,
    id::BoneId,
    pose::{BonePose, Pose},
    skeleton::Skeleton,
    space_conversion::SpaceConversionContext,
};
use serde::{Deserialize, Serialize};

/// Two-bone IK: bends a root/mid/end chain (shoulder, elbow, hand) so the end reaches
/// `target_position`, in character space. Without a target the pose passes through.
///
/// By default the chain is the target bone, its parent and its grandparent. `explicit_chain`
/// names the joints instead (`mid_path`, `root_path`), for rigs with pivot bones between them
/// (WoW's `_p` bones); the bones in between ride along. The optional pins:
/// `target_rotation` sets the end bone's character-space rotation, `pole_target` is a point
/// the middle joint bends toward, and `weight` blends the result over the input pose.
#[derive(Component, Reflect, Clone, Debug, Default, Serialize, Deserialize)]
#[reflect(Component, Default, NodeLike)]
#[type_path = "bevy_animation_graph::builtin_nodes"]
pub struct TwoBoneIKNode {
    #[serde(default)]
    pub explicit_chain: bool,
    #[serde(default)]
    pub target_rotation: bool,
    #[serde(default)]
    pub pole_target: bool,
    #[serde(default)]
    pub weight: bool,
}

impl TwoBoneIKNode {
    pub const IN_TIME: &'static str = "time";
    pub const IN_POSE: &'static str = "pose";
    pub const OUT_POSE: &'static str = "pose";
    pub const TARGETBONE: &'static str = "target_path";
    pub const TARGETPOS: &'static str = "target_position";
    pub const MIDBONE: &'static str = "mid_path";
    pub const ROOTBONE: &'static str = "root_path";
    pub const TARGETROT: &'static str = "target_rotation";
    pub const POLE: &'static str = "pole_target";
    pub const WEIGHT: &'static str = "weight";

    pub fn new() -> Self {
        Self::default()
    }

    fn chain(&self, ctx: &NodeContext, skeleton: &Skeleton, end: BoneId) -> Option<[BoneId; 3]> {
        if self.explicit_chain {
            let mid = ctx.data_back(Self::MIDBONE).ok()?.into_entity_path().ok()?.id();
            let root = ctx.data_back(Self::ROOTBONE).ok()?.into_entity_path().ok()?.id();
            Some([root, mid, end])
        } else {
            let mid = skeleton.parent(&end)?;
            Some([skeleton.parent(&mid)?, mid, end])
        }
    }
}

impl NodeLike for TwoBoneIKNode {
    fn duration(&self, mut ctx: NodeContext) -> Result<(), GraphError> {
        let duration = ctx.duration_back(Self::IN_TIME)?;
        ctx.set_duration_fwd(duration);
        Ok(())
    }

    fn update(&self, mut ctx: NodeContext) -> Result<(), GraphError> {
        if let Ok(input) = ctx.time_update_fwd() {
            ctx.set_time_update_back(Self::IN_TIME, input);
        }

        let mut pose = ctx.data_back(Self::IN_POSE)?.into_pose()?;
        let weight = match self.weight {
            true => ctx.data_back(Self::WEIGHT).and_then(|w| w.as_f32()).unwrap_or(0.0),
            false => 1.0,
        };
        let target = ctx
            .data_back(Self::TARGETBONE)
            .and_then(|t| t.into_entity_path());
        let target_pos = ctx.data_back(Self::TARGETPOS).and_then(|p| p.into_vec3());
        let (Ok(target), Ok(target_pos), true) = (target, target_pos, weight > 0.0) else {
            ctx.set_time(pose.timestamp);
            ctx.set_data_fwd(Self::OUT_POSE, pose);
            return Ok(());
        };
        let target_rot = self
            .target_rotation
            .then(|| ctx.data_back(Self::TARGETROT).and_then(|r| r.as_quat()).ok())
            .flatten();
        let pole = self
            .pole_target
            .then(|| ctx.data_back(Self::POLE).and_then(|p| p.into_vec3()).ok())
            .flatten();
        let Some(skeleton) = ctx
            .graph_context
            .resources
            .skeleton_assets
            .get(&pose.skeleton)
        else {
            return Err(GraphError::SkeletonMissing(ctx.node_id));
        };

        if let Some(chain) = self.chain(&ctx, skeleton, target.id()) {
            let space = ctx.graph_context.space_conversion();
            solve(
                &mut pose,
                skeleton,
                space,
                chain,
                Goal {
                    position: target_pos,
                    rotation: target_rot,
                    pole,
                    weight: weight.min(1.0),
                },
            );
            ctx.graph_context.use_debug_gizmos(|mut gizmos| {
                for bone in chain {
                    gizmos.bone_gizmo(bone, LinearRgba::BLUE, false, skeleton, Some(&pose));
                }
            });
        }
        ctx.set_time(pose.timestamp);
        ctx.set_data_fwd(Self::OUT_POSE, pose);
        Ok(())
    }

    fn spec(&self, mut ctx: SpecContext) -> Result<(), GraphError> {
        ctx.add_input_data(Self::TARGETBONE, DataSpec::EntityPath)
            .add_input_data(Self::TARGETPOS, DataSpec::Vec3);
        if self.explicit_chain {
            ctx.add_input_data(Self::MIDBONE, DataSpec::EntityPath)
                .add_input_data(Self::ROOTBONE, DataSpec::EntityPath);
        }
        if self.target_rotation {
            ctx.add_input_data(Self::TARGETROT, DataSpec::Quat);
        }
        if self.pole_target {
            ctx.add_input_data(Self::POLE, DataSpec::Vec3);
        }
        if self.weight {
            ctx.add_input_data(Self::WEIGHT, DataSpec::F32);
        }
        ctx.add_input_data(Self::IN_POSE, DataSpec::Pose)
            .add_input_time(Self::IN_TIME);
        ctx.add_output_data(Self::OUT_POSE, DataSpec::Pose)
            .add_output_time();
        Ok(())
    }

    fn display_name(&self) -> String {
        "Two Bone IK".into()
    }
}

/// Where the end bone should be, all in character space.
struct Goal {
    position: Vec3,
    rotation: Option<Quat>,
    pole: Option<Vec3>,
    weight: f32,
}

/// Solves in character space and writes back local rotations. Every bone between root and mid
/// (and mid and end) is a descendant that turns rigidly with its joint, so a character-space
/// swing `S` applied at a joint makes any bone under it `S * old` in character space.
fn solve(
    pose: &mut Pose,
    skeleton: &Skeleton,
    space: SpaceConversionContext,
    [root, mid, end]: [BoneId; 3],
    goal: Goal,
) {
    let (Some(root_parent), Some(mid_parent), Some(end_parent)) = (
        skeleton.parent(&root),
        skeleton.parent(&mid),
        skeleton.parent(&end),
    ) else {
        return;
    };
    let char_of = |pose: &Pose, bone| space.character_transform_of_bone(pose, skeleton, bone);
    let (c_root, c_mid, c_end) = (char_of(pose, root), char_of(pose, mid), char_of(pose, end));
    let (c_root_parent, c_mid_parent, c_end_parent) = (
        char_of(pose, root_parent),
        char_of(pose, mid_parent),
        char_of(pose, end_parent),
    );

    let Some((root_swing, mid_swing)) = swings(
        c_root.translation,
        c_mid.translation,
        c_end.translation,
        goal.position,
        goal.pole,
    ) else {
        return;
    };

    let new_root = root_swing * c_root.rotation;
    let new_mid = mid_swing * c_mid.rotation;
    let new_end = goal.rotation.unwrap_or(mid_swing * c_end.rotation);
    let locals = [
        (root, c_root_parent.rotation.inverse() * new_root, c_root_parent.rotation.inverse() * c_root.rotation),
        (mid, (root_swing * c_mid_parent.rotation).inverse() * new_mid, c_mid_parent.rotation.inverse() * c_mid.rotation),
        (end, (mid_swing * c_end_parent.rotation).inverse() * new_end, c_end_parent.rotation.inverse() * c_end.rotation),
    ];
    for (bone, solved, old) in locals {
        if !pose.paths.contains_key(&bone) {
            pose.add_bone(BonePose::default(), bone);
        }
        let index = pose.paths[&bone];
        pose.bones[index].rotation = Some(old.slerp(solved, goal.weight).normalize());
    }
}

/// The character-space swings for the root joint and for the middle joint (the latter including
/// the root's), after littlepolygon's two-bone method. `None` for a degenerate chain.
fn swings(root: Vec3, mid: Vec3, end: Vec3, target: Vec3, pole: Option<Vec3>) -> Option<(Quat, Quat)> {
    const MAX_LEN_OFFSET: f32 = 0.01;
    let upper_len = root.distance(mid);
    let lower_len = mid.distance(end);
    if upper_len < f32::EPSILON || lower_len < f32::EPSILON {
        return None;
    }
    let max_len = upper_len + lower_len - MAX_LEN_OFFSET;

    let to_end = (end - root).normalize_or_zero();
    let to_target_offset = (target - root).clamp_length_max(max_len);
    let to_target_dist = to_target_offset.length();
    if to_target_dist < f32::EPSILON {
        return None;
    }
    let to_target = to_target_offset / to_target_dist;

    // Which way the middle joint bends: toward the pole, or the way the input pose bent it.
    let current_pole = Quat::from_rotation_arc(to_end, to_target)
        * (mid - root).reject_from(to_end).normalize_or_zero();
    let pole_vec = pole
        .map(|p| (p - root).reject_from(to_target).normalize_or_zero())
        .filter(|v| *v != Vec3::ZERO)
        .unwrap_or(current_pole);

    let denom = 2. * upper_len * to_target_dist;
    let cos_angle = ((to_target_dist * to_target_dist + upper_len * upper_len
        - lower_len * lower_len)
        / denom)
        .clamp(-1.0, 1.0);
    let angle = cos_angle.acos();

    let out_end = root + to_target_offset;
    let out_mid = root + upper_len * cos_angle * to_target + upper_len * angle.sin() * pole_vec;

    let root_swing = Quat::from_rotation_arc(
        (mid - root).normalize(),
        (out_mid - root).normalize(),
    );
    let end_after_root = root + root_swing * (end - root);
    let mid_swing = Quat::from_rotation_arc(
        (end_after_root - out_mid).normalize(),
        (out_end - out_mid).normalize(),
    ) * root_swing;
    Some((root_swing, mid_swing))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_end_lands_on_a_reachable_target() {
        let (root, mid, end) = (Vec3::ZERO, Vec3::new(0.0, -0.3, 0.0), Vec3::new(0.0, -0.6, 0.0));
        let target = Vec3::new(0.3, -0.2, 0.2);
        let (root_swing, mid_swing) = swings(root, mid, end, target, Some(Vec3::new(0.0, -0.5, -1.0))).unwrap();
        let new_mid = root + root_swing * (mid - root);
        let new_end = new_mid + mid_swing * (end - mid);
        assert!(new_end.distance(target) < 1e-4, "{new_end} vs {target}");
        assert!((new_mid.distance(root) - 0.3).abs() < 1e-4);
        assert!(new_mid.z < 0.0, "the elbow bends toward the pole: {new_mid}");
    }

    #[test]
    fn an_unreachable_target_straightens_the_chain_toward_it() {
        let (root, mid, end) = (Vec3::ZERO, Vec3::new(0.0, -0.3, 0.0), Vec3::new(0.0, -0.6, 0.0));
        let target = Vec3::new(5.0, 0.0, 0.0);
        let (root_swing, mid_swing) = swings(root, mid, end, target, None).unwrap();
        let new_end = root + root_swing * (mid - root) + mid_swing * (end - mid);
        assert!(new_end.normalize().dot(Vec3::X) > 0.999, "{new_end}");
    }
}
