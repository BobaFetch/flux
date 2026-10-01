//! A sample for comparing highlights with Neovim.
use std::collections::HashMap;

/// A point, with a doc comment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point<T: Copy> {
    pub x: T,
    y: T,
}

const MAX: usize = 10;
static NAME: &str = "flux";

impl<T: Copy + std::fmt::Display> Point<T> {
    pub fn new(x: T, y: T) -> Self {
        Self { x, y }
    }

    fn show(&self) -> String {
        let s = format!("({}, {})", self.x, self.y);
        println!("{s} {:?}", 'c');
        s
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut map: HashMap<&str, i32> = HashMap::new();
    for (i, w) in ["a", "b"].iter().enumerate() {
        map.insert(w, i as i32 * 2 + 0x1F);
    }
    if let Some(v) = map.get("a") && *v > 0 {
        return Err("bad".into()); // an error
    }
    let r = r#"raw "string""#;
    match MAX { 0..=5 => {} _ => unreachable!() }
    Ok(())
}
