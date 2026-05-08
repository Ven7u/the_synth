use forma_engine::generative::load_scene_json;
fn main() {
    let path = std::env::args().nth(1).unwrap_or_default();
    match load_scene_json(&path) {
        Ok(s) => println!("OK — scene: {}, markov: {}", s.name, s.markov.is_some()),
        Err(e) => println!("FAIL: {e}"),
    }
}
