use std::time::Instant;
use zxbs::{build, config, interner, plan, scan, timestamp::timestamp, tracer};

fn make_duration(start: &Instant) -> u64 {
    u64::try_from(start.elapsed().as_micros()).unwrap()
}

async fn run() {
    let config: &'static config::Config = Box::leak(Box::new(config::Config::load_args()));

    let _ = tokio::fs::remove_dir_all(config.build_path).await;
    tokio::fs::create_dir_all(config.build_path).await.unwrap();

    let interner: &'static mut interner::Interner = Box::leak(Box::new(interner::Interner::new()));

    let tracer: &'static mut tracer::Tracer = Box::leak(Box::new(tracer::Tracer::new(timestamp())));

    let beginning = Instant::now();

    let mut t0 = Instant::now();
    let scan_start = make_duration(&beginning);
    let info = scan::scan(config, interner).await;
    let scan_duration = make_duration(&t0);

    t0 = Instant::now();
    let plan_start = make_duration(&beginning);
    let tasks = plan::plan(config, interner, &info);
    let plan_duration = make_duration(&t0);

    t0 = Instant::now();
    let build_start = make_duration(&beginning);
    {
        let build = build::Build::new(
            if config.time_trace {
                Some(tracer)
            } else {
                None
            },
            interner,
            tasks,
            beginning,
        );
        build.await;
    }
    let build_duration = make_duration(&t0);

    if config.time_trace {
        let json = tracer.print_json(
            interner,
            (scan_start, scan_duration),
            (plan_start, plan_duration),
            (build_start, build_duration),
        );
        tokio::fs::write(config.build_path.join("time_trace.json"), json)
            .await
            .unwrap();
    }
}

#[tokio::main]
async fn main() {
    run().await;
}
