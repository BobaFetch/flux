fn control(n: u32) -> u32 {
    let mut total = 0;
    for i in 0..n {
        if i % 2 == 0 {
            total += i;
        } else if i % 3 == 0 {
            total -= 1;
        } else {
            continue;
        }
    }
    while total > 100 {
        total /= 2;
    }
    let label = 'outer: loop {
        loop {
            break 'outer 5;
        }
    };
    let value = if total > 10 { 1 } else { 2 };
    let long = total
        + value
        + label;
    unsafe {
        std::ptr::null::<u8>();
    }
    long
}

impl Iterator for Counter {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        self.count += 1;
        Some(self.count)
    }
}

trait Shape {
    fn area(&self) -> f64;
    fn name(&self) -> String {
        String::from("shape")
    }
}
