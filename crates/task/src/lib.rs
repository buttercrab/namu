use kanal::{Receiver, Sender};
use serde::{Deserialize, Serialize};

type ItemId = usize;

pub trait Task<'a> {
    type Input: Serialize + Deserialize<'a> + Send + Sync + Clone;
    type Output: Serialize + Deserialize<'a> + Send + Sync + Clone;

    fn run(
        &mut self,
        input: Receiver<(ItemId, Self::Input)>,
        output: Sender<(ItemId, Self::Output)>,
    );
}

pub trait SingleTask<'a>: Task<'a> {
    fn call(&mut self, input: Self::Input) -> Self::Output;

    fn run(
        &mut self,
        input: Receiver<(ItemId, Self::Input)>,
        output: Sender<(ItemId, Self::Output)>,
    ) {
        while let Ok((id, x)) = input.recv() {
            let y = self.call(x);
            let _ = output.send((id, y));
        }
    }
}
