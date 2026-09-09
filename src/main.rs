use zxbs::{build, config, interner, plan, scan, timestamp::timestamp, tracer};

async fn run() {
    let config: &'static config::Config = Box::leak(Box::new(config::Config::load_args()));

    let _ = tokio::fs::remove_dir_all(config.build_path).await;
    tokio::fs::create_dir_all(config.build_path).await.unwrap();

    let interner: &'static mut interner::Interner = Box::leak(Box::new(interner::Interner::new()));

    let tracer: &'static mut tracer::Tracer = Box::leak(Box::new(tracer::Tracer::new(timestamp())));

    let info = scan::scan(config, interner).await;

    let scan_timestamp = timestamp();

    let tasks = plan::plan(config, interner, &info);

    let plan_timestamp = timestamp();

    {
        let build = build::Build::new(
            if config.time_trace {
                Some(tracer)
            } else {
                None
            },
            interner,
            tasks,
        );
        build.await;
    }

    let build_timestamp = timestamp();

    if config.time_trace {
        let json = tracer.print_json(interner, scan_timestamp, plan_timestamp, build_timestamp);
        tokio::fs::write(config.build_path.join("time_trace.json"), json)
            .await
            .unwrap();
    }
}

#[tokio::main]
async fn main() {
    run().await;
}
