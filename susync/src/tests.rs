use super::*;

#[tokio::test]
async fn return_value() {
    let (fut, comp) = create();

    tokio::spawn(async move {
        let res = fut.await;
        assert!(res.is_ok());
    });

    tokio::spawn(async move {
        assert!(comp.complete(()));
    });
}

#[tokio::test]
async fn drop_handle() {
    let (fut, comp) = create::<()>();
    
    let inner_comp = comp.clone();
    tokio::spawn(async move {
        std::thread::sleep(std::time::Duration::from_millis(100));
        drop(inner_comp);
    });

    drop(comp);
    let res = fut.await;
    assert!(res.is_err(), "expected Err, got Ok result");
}

#[tokio::test]
async fn complete_after_drop() {
    let (fut, comp) = create::<()>();
    
    let inner_comp = comp.clone();
    tokio::spawn(async move {
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(inner_comp.complete(()));
    });

    drop(comp);
    let res = fut.await;
    assert!(res.is_ok(), "expected Ok, got Err result");
}

#[tokio::test]
async fn complete_after_await() {
    let (fut, comp) = create::<()>();
    
    let inner_comp = comp.clone();
    tokio::spawn(async move {
        std::thread::sleep(std::time::Duration::from_millis(100));
        inner_comp.complete(());
    });

    let res = fut.await;
    assert!(res.is_ok(), "expected Ok, got Err result");
    assert!(!comp.complete(()));
}

#[tokio::test]
async fn many_drops_race() {
    let (fut, comp) = create::<()>();

    for _ in 0..10 {
        let _ = comp.clone();
    }
    drop(comp);

    let res= fut.await;
    assert!(res.is_err());
}

#[tokio::test]
async fn many_drops_race_async() {
    let (fut, comp) = create::<()>();

    for _ in 0..10 {
        let inner_comp = comp.clone();
        tokio::spawn(async move {
            drop(inner_comp);
        });
    }
    drop(comp);

    let res= fut.await;
    assert!(res.is_err());
}

#[tokio::test]
async fn many_handles_race() {
    let (fut, comp) = create::<()>();

    for i in 0..10 {
        let inner_comp = comp.clone();
        if i == 5 {
            assert!(inner_comp.complete(()));
        }
    }
    drop(comp);

    let res= fut.await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn many_handles_race_async() {
    let (fut, comp) = create::<()>();

    for i in 0..10 {
        let inner_comp = comp.clone();
        tokio::spawn(async move {
            if i == 5 {
                assert!(inner_comp.complete(()));
            }
        });
    }
    drop(comp);

    let res= fut.await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn complete_before_await() {
    let (fut, comp) = create::<()>();

    assert!(comp.complete(()));
    tokio::spawn(async move {
        fut.await.unwrap();
    });
}

#[tokio::test]
async fn complete_early_err() {
    let (fut, comp) = create();

    let inner_comp = comp.clone();
    let res = mock_callback_func_early_err(move |res| {
        assert!(!inner_comp.complete(Ok(res)));
    });
    if let Err(res) = res {
        assert!(comp.complete(Err(res)));
    }

    let res = fut.await;
    assert!(res.is_ok());
    let res = res.unwrap();
    assert!(res.is_err());
}

fn mock_callback_func(cb: impl FnOnce(()) + Send + 'static) -> Result<(), ()> {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        cb(());
    });
    Ok(())
}

#[tokio::test]
async fn suspend_func() {
    let fut = suspend(|comp| {
        let _ = mock_callback_func(move |res| {
            assert!(comp.complete(res));
        });
    });

    let res = fut.await;
    assert!(res.is_ok());
}

fn mock_callback_func_early_err(cb: impl FnOnce(()) + Send + 'static) -> Result<(), ()> {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        cb(());
    });
    Err(())
}

#[tokio::test]
async fn suspend_func_early_err() {
    let fut = suspend(|comp| {
        let inner_comp = comp.clone();
        let res = mock_callback_func_early_err(move |res| {
            assert!(!inner_comp.complete(Ok(res)));
        });
        if let Err(res) = res {
            assert!(comp.complete(Err(res)));
        }
    });

    let res = fut.await;
    assert!(res.is_ok());
    let res = res.unwrap();
    assert!(res.is_err());
}

#[tokio::test]
async fn complete_multiple_sequencially() {
    let (future, handle1) = create();
    let handle2 = handle1.clone();
    let handle3 = handle1.clone();
    // successfully sends the value to complete on both
    assert!(handle1.complete(1));
    assert!(handle2.complete(2));
    
    let result = future.await.unwrap();
    // unsuccessfully tries to complete a future that completed
    assert!(!handle3.complete(3));
    assert_eq!(result, 1);
}
