use std::time::Instant;
use zxbs::{build, cache, config, directory, interner, plan, scan, timestamp::timestamp, tracer};

fn make_duration(start: &Instant) -> u64 {
    u64::try_from(start.elapsed().as_micros()).unwrap()
}

async fn run() {
    let (config, subcommand) = config::Config::parse_cli();

    let config: &'static config::Config = Box::leak(Box::new(config));

    let interner: &'static mut interner::Interner = Box::leak(Box::new(interner::Interner::new()));

    let start_timestamp = timestamp();

    let tracer: &'static mut tracer::Tracer =
        Box::leak(Box::new(tracer::Tracer::new(start_timestamp)));

    let mut cache_state = cache::load(config.cache_path).await;

    let beginning = Instant::now();

    let mut t0 = Instant::now();
    let scan_start = make_duration(&beginning);
    let info = scan::scan(config, interner).await;
    let scan_duration = make_duration(&t0);

    info.check_subcommand(interner, &subcommand);

    directory::setup_build_dir(config, &info, interner).await;

    t0 = Instant::now();
    let plan_start = make_duration(&beginning);
    let tasks = plan::plan(
        config,
        &subcommand,
        interner,
        &info,
        &mut cache_state,
        start_timestamp,
    );
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

    cache::finish(cache_state, config.cache_path, start_timestamp).await;

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
