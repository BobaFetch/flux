fn chains(items: &[i32]) -> Vec<i32> {
    let total: i32 = items
        .iter()
        .filter(|x| **x > 0)
        .map(|x| x * 2)
        .sum();
    let names = items.iter().map(|i| i.to_string()).collect::<Vec<_>>();
    let result = items
        .iter()
        .copied()
        .collect();
    let _ = (total, names);
    result
}

fn builder() -> String {
    String::new()
        .chars()
        .rev()
        .collect()
}
