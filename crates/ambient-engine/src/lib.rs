pub mod patch;
pub mod engine;
pub use patch::AmbientPatch;
pub use engine::{
    AmbientEngine,
    MacroSetKind,
    MacroParam,
    MacroTarget,
    Scene,
    SceneGlobal,
    SceneMacro,
    SceneTrack,
    MACRO_COUNT,
    ACTIVE_MACRO_KNOBS,
    TrackState,
    TRACK_COUNT,
    VOICE_COUNT,
    save_scene_json,
    load_scene_json,
};
