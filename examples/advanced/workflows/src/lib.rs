use add::add;
use embed_batch::embed_batch;
use id_range::id_range;
use is_even::is_even;
use less_than::less_than;
use maybe_fail::maybe_fail;
use namu::prelude::*;
use normalize::normalize;
use score::score;

#[workflow]
pub fn etl_pipeline() -> i32 {
    let ids = id_range(1, 6);
    let cleaned = normalize(ids);
    let scored = score(cleaned);

    let mut bonus = 0;
    let mut step = 0;
    while less_than(step, 3) {
        bonus = add(bonus, step);
        step = add(step, 1);
    }

    if is_even(scored) {
        add(scored, bonus)
    } else {
        scored
    }
}

#[workflow]
pub fn ml_pipeline() -> i32 {
    let tokens = id_range(1, 10);
    let embedded = embed_batch(tokens);
    let scored = score(embedded);

    if is_even(scored) {
        add(scored, 1)
    } else {
        scored
    }
}

#[workflow]
pub fn media_pipeline() -> i32 {
    let frames = id_range(1, 6);
    let processed = maybe_fail(frames);
    score(processed)
}
