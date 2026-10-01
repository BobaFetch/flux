enum Shape {
    Circle { r: f64 },
    Rect(f64, f64),
    Empty,
}

fn area(s: &Shape) -> f64 {
    match s {
        Shape::Circle { r } => 3.14 * r * r,
        Shape::Rect(w, h) => {
            let a = w * h;
            a
        }
        Shape::Empty => 0.0,
    }
}

fn describe(n: i32) -> &'static str {
    match n {
        0 => "zero",
        1 | 2 => "small",
        x if x < 0 => {
            "negative"
        }
        _ => "big",
    }
}

fn nested(v: Option<Result<i32, String>>) -> i32 {
    if let Some(r) = v {
        match r {
            Ok(n) => n,
            Err(e) => {
                eprintln!("{e}");
                -1
            }
        }
    } else {
        0
    }
}
