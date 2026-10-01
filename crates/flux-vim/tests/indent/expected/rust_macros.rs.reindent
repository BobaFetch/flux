macro_rules! square {
    ($x:expr) => {
        $x * $x
    };
}

fn macros() {
    let v = vec![
        1,
        2,
        3,
    ];
    println!(
        "{} {}",
        v.len(),
        square!(2)
    );
    assert_eq!(
        v,
        [1, 2, 3]
    );
    let s = format!("{}{}", "a",
        "b");
    let _ = s;
}
