//! A head that looks where it is told, from wherever its clip put it.
//!
//! The game says where the face should point (a direction in character space, with a weight),
//! and how a resting face should pitch; the node measures the face the incoming pose actually
//! has and turns the head (and part of it the neck) to take it there, about the character's up
//! and the face's right. Measuring inside the evaluation is the point: the turn is relative to
//! this very pose, so a clip that hangs its head, nods or sways keeps doing so around the aim,
//! and nothing outside the graph has to guess which pose its last turn landed on.

use bevy::ecs::{component::Component, reflect::ReflectComponent};
use bevy::prelude::*;
use bevy_animation_graph_core::{
    animation_node::{NodeLike, ReflectNodeLike},
    context::{new_context::NodeContext, spec_context::SpecContext},
    edge_data::DataSpec,
    errors::GraphError,
    id::BoneId,
    pose::{BonePose, Pose},
    skeleton::Skeleton,
};

#[derive(Component, Reflect, Clone, Debug)]
#[reflect(Component, Default, NodeLike)]
#[type_path = "bevy_animation_graph::builtin_nodes"]
pub struct LookAtNode {
    /// The face's direction in the head bone's frame.
    pub face: Vec3,
    /// The fraction of the turn the neck (the head's parent) takes; the head takes the rest.
    pub neck_share: f32,
}

impl Default for LookAtNode {
    fn default() -> Self {
        Self {
            face: Vec3::NEG_Z,
            neck_share: 0.4,
        }
    }
}

impl LookAtNode {
    pub const IN_POSE: &'static str = "pose";
    pub const IN_TIME: &'static str = "time";
    pub const HEAD: &'static str = "head";
    pub const DIRECTION: &'static str = "direction";
    pub const WEIGHT: &'static str = "weight";
    pub const REST_PITCH: &'static str = "rest_pitch";
    pub const LEVEL: &'static str = "level";
    pub const OUT_POSE: &'static str = "pose";
}

/// Yaw (about +Y, positive turning -Z toward -X) and pitch (positive up) of a direction, in a
/// frame whose forward is -Z.
pub fn yaw_pitch(d: Vec3) -> Vec2 {
    Vec2::new(
        f32::atan2(-d.x, -d.z),
        f32::atan2(d.y, Vec2::new(d.x, d.z).length()),
    )
}

/// The rotation that turns -Z to `yaw_pitch`.
pub fn turn(yaw_pitch: Vec2) -> Quat {
    Quat::from_euler(EulerRot::YXZ, yaw_pitch.x, yaw_pitch.y, 0.0)
}

/// What a look asks of a head.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    /// Where the face should point (character space); zero for nowhere in particular.
    pub direction: Vec3,
    /// How much of the way from rest to `direction` the face goes (0..1).
    pub weight: f32,
    /// A resting face's pitch over level (rad).
    pub rest_pitch: f32,
    /// How much a resting face takes `rest_pitch` rather than the clip's own (0..1).
    pub level: f32,
}

/// The neck's and head's new local rotations: `parent` is the neck's parent's character-space
/// rotation, `neck` and `head` their local rotations in the pose, `face` the face's direction in
/// the head's frame. The face ends at `look`'s yaw and pitch (character space), the neck taking
/// `share` of the turn.
pub fn aim(face: Vec3, share: f32, parent: Quat, neck: Quat, head: Quat, look: Look) -> (Quat, Quat) {
    let neck_char = parent * neck;
    let clip = yaw_pitch(neck_char * head * face);
    let rest = Vec2::new(clip.x, clip.y + (look.rest_pitch - clip.y) * look.level.clamp(0.0, 1.0));
    let goal = if look.direction.length_squared() > 1e-12 {
        rest + (yaw_pitch(look.direction) - rest) * look.weight.clamp(0.0, 1.0)
    } else {
        rest
    };
    let back = turn(clip).inverse();
    let total = turn(goal) * back;
    let neck_turn = turn(clip + (goal - clip) * share) * back;
    let head_turn = total * neck_turn.inverse();
    // Each turn in its bone's parent frame, laid on the local rotation (the head's parent being
    // the neck once it has turned).
    let neck_local = parent.inverse() * neck_turn * parent * neck;
    let turned_neck = neck_turn * neck_char;
    let head_local = turned_neck.inverse() * head_turn * turned_neck * head;
    (neck_local, head_local)
}

fn local_rotation(pose: &Pose, skeleton: &Skeleton, bone: BoneId) -> Quat {
    pose.paths
        .get(&bone)
        .and_then(|i| pose.bones.get(*i))
        .and_then(|b| b.rotation)
        .or_else(|| skeleton.default_transforms(bone).map(|t| t.local.rotation))
        .unwrap_or(Quat::IDENTITY)
}

fn set_rotation(pose: &mut Pose, bone: BoneId, rotation: Quat) {
    let mut bone_pose = pose
        .paths
        .get(&bone)
        .and_then(|i| pose.bones.get(*i).cloned())
        .unwrap_or_else(BonePose::default);
    bone_pose.rotation = Some(rotation);
    pose.add_bone(bone_pose, bone);
}

impl NodeLike for LookAtNode {
    fn update(&self, mut ctx: NodeContext) -> Result<(), GraphError> {
        if let Ok(input) = ctx.time_update_fwd() {
            ctx.set_time_update_back(Self::IN_TIME, input);
        }
        let mut pose = ctx.data_back(Self::IN_POSE)?.into_pose()?;
        // Unset (an editor preview), the pose passes through.
        let (Ok(head), Ok(direction)) = (
            ctx.data_back(Self::HEAD).and_then(|t| t.into_entity_path()),
            ctx.data_back(Self::DIRECTION).and_then(|d| d.as_vec3()),
        ) else {
            ctx.set_time(pose.timestamp);
            ctx.set_data_fwd(Self::OUT_POSE, pose);
            return Ok(());
        };
        let f32_or = |ctx: &NodeContext, pin: &str, or: f32| {
            ctx.data_back(pin).and_then(|v| v.as_f32()).unwrap_or(or)
        };
        let look = Look {
            direction,
            weight: f32_or(&ctx, Self::WEIGHT, 0.0),
            rest_pitch: f32_or(&ctx, Self::REST_PITCH, 0.0),
            level: f32_or(&ctx, Self::LEVEL, 0.0),
        };
        let Some(skeleton) = ctx
            .graph_context
            .resources
            .skeleton_assets
            .get(&pose.skeleton)
        else {
            return Err(GraphError::SkeletonMissing(ctx.node_id));
        };
        let head = head.id();
        let (Some(neck), true) = (skeleton.parent(&head), self.face != Vec3::ZERO) else {
            ctx.set_time(pose.timestamp);
            ctx.set_data_fwd(Self::OUT_POSE, pose);
            return Ok(());
        };
        let parent = skeleton.parent(&neck).map_or(Quat::IDENTITY, |p| {
            ctx.graph_context
                .space_conversion()
                .character_transform_of_bone(&pose, skeleton, p)
                .rotation
        });
        let (neck_local, head_local) = aim(
            self.face,
            self.neck_share,
            parent,
            local_rotation(&pose, skeleton, neck),
            local_rotation(&pose, skeleton, head),
            look,
        );
        set_rotation(&mut pose, neck, neck_local);
        set_rotation(&mut pose, head, head_local);

        ctx.set_time(pose.timestamp);
        ctx.set_data_fwd(Self::OUT_POSE, pose);
        Ok(())
    }

    fn spec(&self, mut ctx: SpecContext) -> Result<(), GraphError> {
        ctx //
            .add_input_data(Self::IN_POSE, DataSpec::Pose)
            .add_input_data(Self::HEAD, DataSpec::EntityPath)
            .add_input_data(Self::DIRECTION, DataSpec::Vec3)
            .add_input_data(Self::WEIGHT, DataSpec::F32)
            .add_input_data(Self::REST_PITCH, DataSpec::F32)
            .add_input_data(Self::LEVEL, DataSpec::F32)
            .add_input_time(Self::IN_TIME);
        ctx //
            .add_output_data(Self::OUT_POSE, DataSpec::Pose)
            .add_output_time();
        Ok(())
    }

    fn display_name(&self) -> String {
        "◉ Look at".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face_of(parent: Quat, neck: Quat, head: Quat) -> Vec3 {
        parent * neck * head * Vec3::NEG_Z
    }

    /// A spine leaning forward, a neck and a head hanging 25 degrees: the face ends where it is
    /// told, the neck takes its share, and a weightless look leaves the clip alone.
    #[test]
    fn the_face_points_where_it_is_told() {
        let parent = Quat::from_rotation_x(-0.2);
        let neck = Quat::from_rotation_x(0.1) * Quat::from_rotation_y(0.05);
        let head = Quat::from_rotation_x(-25f32.to_radians());
        let dir = Vec3::new(-0.6, 0.15, -1.0);
        let look = Look {
            direction: dir,
            weight: 1.0,
            rest_pitch: 0.0,
            level: 1.0,
        };
        let (n, h) = aim(Vec3::NEG_Z, 0.4, parent, neck, head, look);
        assert!(face_of(parent, n, h).angle_between(dir) < 1e-4);
        // The neck turned some of the way, not all of it.
        let neck_face = yaw_pitch(parent * n * Vec3::NEG_Z);
        let before = yaw_pitch(parent * neck * Vec3::NEG_Z);
        assert!((neck_face.x - before.x).abs() > 0.1);
        assert!((neck_face.x - before.x).abs() < yaw_pitch(dir).x.abs() * 0.8);

        // At rest, leveled: the clip's yaw at the rest pitch.
        let rest = Look {
            direction: Vec3::ZERO,
            weight: 0.0,
            rest_pitch: -0.05,
            level: 1.0,
        };
        let clip = yaw_pitch(face_of(parent, neck, head));
        let (n, h) = aim(Vec3::NEG_Z, 0.4, parent, neck, head, rest);
        let f = yaw_pitch(face_of(parent, n, h));
        assert!((f - Vec2::new(clip.x, -0.05)).length() < 1e-4);

        // Nothing asked: the pose as it was.
        let none = Look {
            level: 0.0,
            ..rest
        };
        let (n, h) = aim(Vec3::NEG_Z, 0.4, parent, neck, head, none);
        assert!(n.angle_between(neck) < 2e-3 && h.angle_between(head) < 2e-3);
    }
}
