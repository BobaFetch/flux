fn closures() {
    let add = |a: i32, b: i32| a + b;
    let complex = |x: i32| {
        let y = x * 2;
        y + 1
    };
    let v: Vec<i32> = (0..10)
        .map(|x| {
            x * x
        })
        .collect();
    std::thread::spawn(move || {
        println!("{}", add(1, 2));
    });
    let _ = (complex, v);
}

fn call_with_args(
    first: i32,
    second: &str,
    third: Option<bool>,
) -> bool {
    third.unwrap_or(first > 0 && !second.is_empty())
}

fn short(a: i32,
         b: i32) -> i32 {
    a + b
}
