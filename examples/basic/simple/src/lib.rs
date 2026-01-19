use add::add;
use is_less::is_less;
use namu::prelude::*;

#[workflow]
pub fn simple() -> i32 {
    let mut a = 0;
    let mut b = 1;

    while is_less(a, 10) {
        let c = add(a, b);
        a = b;
        b = c;
    }

    b
}
