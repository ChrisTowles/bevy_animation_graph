//! `.animgraph.bsn` and `.fsm.bsn` files, spawned as scenes, build the graphs and state machines
//! they describe.

use bevy::{asset::AssetPlugin, prelude::*};
use bevy_animation_graph_builtin_nodes::BuiltinNodesPlugin;
use bevy_animation_graph_core::{
    animation_clip::GraphClip,
    animation_graph::{
        AnimationGraph, SourcePin, TargetPin,
        bsn::{AnimGraph, Links, LinksTemplate, NodePosition, graph_from_world, node_id, spawn_document},
    },
    state_machine::high_level::{
        StateMachine,
        bsn::{
            AnimFsm, AnimFsmTemplate, AnimState, AnimTransition, AnimTransitionTemplate,
            fsm_from_world, state_id,
        },
    },
};

/// zero's one-shot `jump` state: the clip plays, and its finish event becomes a transition
/// request on `driver_events`.
const JUMP: &str = r#"
#jump
AnimGraph { outputs: [Time, Data("pose", Pose), Data("driver_events", EventQueue)] }
Links([
    Link(Data("pose"), Node(#clip, "pose")),
    Link(Time(""), NodeTime(#clip)),
    Link(Data("driver_events"), Node(#"on finish", "events")),
])
Children [
    #clip
    ClipNode { clip: "anim/ual/Jump_Start.anim.ron" }
    NodePosition(40.0, 40.0)
    --
    #"on finish"
    MapEventsNode { map_from: AnimationClipFinished, map_to: TransitionToStateLabel("airborne") }
    Links([Link(Data("events"), Node(#clip, "events"))])
    NodePosition(330.0, 140.0)
]
"#;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((TaskPoolPlugin::default(), AssetPlugin::default(), BuiltinNodesPlugin))
        .init_asset::<GraphClip>()
        .init_asset::<AnimationGraph>()
        .init_asset::<StateMachine>()
        .register_type::<AnimGraph>()
        .register_type::<Links>()
        .register_type::<LinksTemplate>()
        .register_type::<NodePosition>()
        .register_type::<AnimFsm>()
        .register_type::<AnimFsmTemplate>()
        .register_type::<AnimState>()
        .register_type::<AnimTransition>()
        .register_type::<AnimTransitionTemplate>()
        .register_type::<Children>()
        .register_type::<ChildOf>()
        .register_type::<Name>();
    app
}

fn spawn(app: &App, text: &str) -> Result<(World, Entity), String> {
    let registry = app.world().resource::<AppTypeRegistry>().0.clone();
    let server = app.world().resource::<AssetServer>().clone();
    let mut handles = |type_id, path| server.load_builder().load_erased(type_id, path);
    spawn_document(text, "test.bsn", &registry, &server, &mut handles)
}

fn graph(app: &App, text: &str) -> Result<AnimationGraph, String> {
    let (world, root) = spawn(app, text)?;
    graph_from_world(&world, root, &app.world().resource::<AppTypeRegistry>().read())
}

#[test]
fn a_bsn_graph_builds_its_nodes_edges_and_pins() {
    let graph = graph(&app(), JUMP).expect("builds");

    assert_eq!(graph.nodes.len(), 2);
    let clip = node_id("clip");
    let finish = node_id("on finish");
    assert_eq!(graph.nodes[&clip].name, "clip");
    assert_eq!(
        graph.edges_inverted[&TargetPin::OutputData("pose".into())],
        SourcePin::NodeData(clip, "pose".into())
    );
    assert_eq!(
        graph.edges_inverted[&TargetPin::OutputTime],
        SourcePin::NodeTime(clip)
    );
    assert_eq!(
        graph.edges_inverted[&TargetPin::NodeData(finish, "events".into())],
        SourcePin::NodeData(clip, "events".into())
    );
    assert!(graph.io_spec.has_output_time());
    assert_eq!(
        graph.editor_metadata.node_positions[&finish],
        Vec2::new(330.0, 140.0)
    );
}

#[test]
fn graph_inputs_keep_their_order_and_defaults() {
    let doc = r#"
#g
AnimGraph {
    inputs: [
        Data { pin: Passthrough("speed"), spec: F32, default: Some(F32(1.5)) },
        Data { pin: FromFsmSource("pose"), spec: Pose },
        Time(FromFsmSource("time")),
    ],
    outputs: [Data("pose", Pose)],
}
Links([Link(Data("pose"), Input(FromFsmSource("pose")))])
"#;
    let graph = graph(&app(), doc).expect("builds");
    let inputs = graph.io_spec.sorted_inputs();
    assert_eq!(inputs.len(), 3, "{inputs:?}");
    assert_eq!(graph.default_data.len(), 1);
}

#[test]
fn a_node_without_a_node_component_is_refused() {
    let doc = "#g\nAnimGraph { }\nChildren [\n    #lost\n    NodePosition(1.0, 2.0)\n]\n";
    let err = graph(&app(), doc).unwrap_err();
    assert!(err.contains("lost"), "{err}");
}

const LOCOMOTION: &str = r#"
#locomotion
AnimFsm { start: #grounded, inputs: [Data("speed", F32)], outputs: [Time, Data("pose", Pose)] }
Children [
    #grounded
    AnimState { }
    NodePosition(0.0, 0.0)
    --
    #jump
    AnimState { enter: Some(TransitionData { kind: Graph { graph: "t.animgraph.bsn", timed: Some(0.08) }, reset_target_state: true }) }
    NodePosition(330.0, -150.0)
    --
    AnimTransition { from: #jump, to: #grounded }
]
"#;

#[test]
fn a_bsn_state_machine_builds_its_states_and_transitions() {
    let app = app();
    let (world, root) = spawn(&app, LOCOMOTION).expect("spawns");
    let fsm = fsm_from_world(&world, root).expect("builds");

    assert_eq!(fsm.states.len(), 2);
    assert_eq!(fsm.start_state, state_id("grounded"));
    let jump = &fsm.states[&state_id("jump")];
    assert_eq!(jump.label, "jump");
    let enter = jump.state_transition.as_ref().expect("jump has an entry transition");
    assert!(enter.reset_target_state);
    assert_eq!(fsm.transitions.len(), 1);
    let t = fsm.transitions.values().next().unwrap();
    assert_eq!((t.source, t.target), (state_id("jump"), state_id("grounded")));
    assert_eq!(fsm.editor_metadata.states[&state_id("jump")], Vec2::new(330.0, -150.0));
}

#[test]
fn a_start_state_that_is_not_a_state_is_refused() {
    let doc = LOCOMOTION.replace("start: #grounded", "start: #flying");
    let err = spawn(&app(), &doc).map(|_| ()).unwrap_err();
    assert!(err.contains("flying"), "{err}");
}
