#![allow(unused_variables)]

use graph::workflow;
use namu as graph;

#[workflow]
fn immutable_assign() {
    let x = 1;
    x = 2;
}

fn main() {}
