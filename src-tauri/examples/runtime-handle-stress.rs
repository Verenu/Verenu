fn main() {
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    tauri::Builder::default()
        .setup(|app| {
            tauri::WebviewWindowBuilder::new(
                app,
                "runtime-stress",
                tauri::WebviewUrl::External("about:blank".parse()?),
            )
            .visible(false)
            .build()?;
            let handle = std::sync::Arc::new(app.handle().clone());
            std::thread::spawn(move || {
                let workers: Vec<_> = (0..8)
                    .map(|_| {
                        let handle = std::sync::Arc::clone(&handle);
                        std::thread::spawn(move || {
                            for _ in 0..50_000 {
                                std::hint::black_box(handle.as_ref().clone());
                            }
                        })
                    })
                    .collect();
                for worker in workers {
                    worker.join().expect("Native handle worker");
                }
                println!("400000 concurrent native handle clones and drops completed");
                handle.exit(0);
            });
            Ok(())
        })
        .run(context)
        .expect("Native runtime stress session");
}
