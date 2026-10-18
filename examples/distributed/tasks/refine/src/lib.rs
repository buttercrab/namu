//! Refines a label, one letter per step, like a decoder generating a token per step. A
//! continuous batch task: words join and leave the session between steps, so a short word never
//! waits for a long one.

use std::collections::HashMap;

#[derive(Default)]
struct Refiner {
    /// Each word in the session, with the letters it has left.
    active: HashMap<u64, (String, usize)>,
}

impl namu_guest::Continuous for Refiner {
    type In = String;
    type Out = String;

    fn step(&mut self, admit: Vec<(u64, String)>) -> Vec<(u64, Result<String, String>)> {
        for (id, word) in admit {
            let letters = word.len();
            self.active.insert(id, (word, letters));
        }
        let mut done = Vec::new();
        self.active.retain(|&id, (word, left)| {
            *left = left.saturating_sub(1);
            if *left > 0 {
                return true;
            }
            done.push((id, Ok(word.to_uppercase())));
            false
        });
        done
    }
}

namu_guest::continuous!(Refiner);
