pub mod engine;
pub mod generators;
pub mod markov;
pub mod patch;
pub use engine::{
    load_scene_json, migrate_patch_json_to_scene_json, save_scene_json, scene_from_single_patch,
    AmbientEngine, HarmonicSlot, MacroParam, MacroSetKind, MacroTarget, MarkovScene, Scene,
    SceneGlobal, SceneMacro, SceneTrack, TrackState, ACTIVE_MACRO_KNOBS, MACRO_COUNT, TRACK_COUNT,
    VOICE_COUNT,
};
pub use generators::{
    EuclideanGen, EuclideanShared, GenEvent, GenerativeMode, ProbTableGen, ProbTableShared,
    EUCLIDEAN_MAX_STEPS, PROB_TABLE_MAX_STEPS,
};
pub use markov::{
    EffectsTargets,
    // Chains
    HarmonicChain,
    // State enums
    HarmonicFunction,
    // Matrix types
    HarmonicMatrix,
    // Engine
    MarkovEngine,
    MarkovEngineShared,
    MarkovVoice,
    MelodicChain,
    MelodicMatrix,
    // Mood
    MoodBlend,
    MoodSet,
    // Phrase
    PhraseCounter,
    PhraseEvents,
    ResolvedState,
    RhythmicChain,
    RhythmicMatrix,
    RhythmicState,
    Scale,
    // Timeline
    Timeline,
    TimelineSection,
    TimelineStatus,
    VoiceEvent,
    VoiceRole,
    ALL_MOODS,
    HARMONIC_STATES,
    // Launchpad
    LAUNCHPAD_COLS,
    MELODIC_STATES,
    MOOD_CALM,
    MOOD_COSMIC,
    MOOD_DARK,
    MOOD_EUPHORIC,
    MOOD_GRAVITY,
    MOOD_TENSE,
    N_MOODS,
    PHRASE_BOUNDARY_HARMONIC,
    RHYTHMIC_STATES,
};
pub use patch::AmbientPatch;
pub use synth_common::{BeatClock, BeatClockShared, BeatEvents, BeatPosition};
