pub mod generators;
pub mod markov;
pub mod patch;
pub mod engine;
pub use synth_common::{BeatClock, BeatClockShared, BeatEvents, BeatPosition};
pub use generators::{
    EuclideanGen, EuclideanShared, GenEvent, GenerativeMode,
    ProbTableGen, ProbTableShared,
    EUCLIDEAN_MAX_STEPS, PROB_TABLE_MAX_STEPS,
};
pub use markov::{
    // Engine
    MarkovEngine, MarkovEngineShared, MarkovVoice, VoiceEvent,
    // Chains
    HarmonicChain, RhythmicChain, MelodicChain,
    // State enums
    HarmonicFunction, RhythmicState, VoiceRole, Scale,
    // Phrase
    PhraseCounter, PhraseEvents,
    // Mood
    MoodBlend, MoodSet,
    ALL_MOODS, N_MOODS,
    MOOD_CALM, MOOD_TENSE, MOOD_DARK, MOOD_EUPHORIC, MOOD_COSMIC, MOOD_GRAVITY,
    PHRASE_BOUNDARY_HARMONIC,
    // Matrix types
    HarmonicMatrix, RhythmicMatrix, MelodicMatrix,
    HARMONIC_STATES, RHYTHMIC_STATES, MELODIC_STATES,
    // Launchpad
    LAUNCHPAD_COLS,
};
pub use patch::AmbientPatch;
pub use engine::{
    AmbientEngine,
    MarkovScene, HarmonicSlot,
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
    scene_from_single_patch,
    migrate_patch_json_to_scene_json,
};
