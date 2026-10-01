use std::fmt::Debug;

fn show<T>(value: T) -> String
where
    T: Debug + Clone,
{
    format!("{value:?}")
}

fn pair<A, B>(a: A, b: B) -> (A, B)
where A: Clone,
      B: Clone,
{
    (a, b)
}

struct Wrapper<T>
where
    T: Debug,
{
    inner: T,
}

impl<T> Wrapper<T>
where
    T: Debug,
{
    fn get(&self) -> &T {
        &self.inner
    }
}
