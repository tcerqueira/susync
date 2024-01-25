use susync_macros::suspend;

fn test_fn_empty(_a: i32, func: impl FnOnce()) {
    func();
}

// fn test_fn(a: i32, func: impl FnOnce(i32, &str)) {
//     func(a, "test");
// }

#[tokio::test]
async fn macro_test_empty() {
    let fut = suspend!(test_fn_empty(42, || {}));

    let _ = fut.await;
}

// #[tokio::test]
// async fn macro_test() {
//     let fut = suspend!(test_fn(42, |a, s| {
//         println!("{}, {}", a, s);
//     }));

//     fut.await;
// }