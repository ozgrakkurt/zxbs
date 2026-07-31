use std::time::Instant;

use zxbs::{build, config, interner, plan, scan, tracer};

async fn run() {
    let config: &'static config::Config = Box::leak(Box::new(config::Config::load_args()));

    let _ = tokio::fs::remove_dir_all(config.build_path).await;
    tokio::fs::create_dir_all(config.build_path).await.unwrap();

    let interner: &'static mut interner::Interner = Box::leak(Box::new(interner::Interner::new()));

    let tracer: &'static mut tracer::Tracer = Box::leak(Box::new(tracer::Tracer::new()));

    let start_time = Instant::now();

    let mut t0 = Instant::now();
    let info = scan::scan(config, interner).await;

    if config.time_trace {
        eprintln!("scan took {}ms", t0.elapsed().as_millis());
    }
    t0 = Instant::now();

    let tasks = plan::plan(config, interner, &info);
    if config.time_trace {
        eprintln!("plan took {}ms", t0.elapsed().as_millis());
    }
    t0 = Instant::now();

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

    if config.time_trace {
        eprintln!("build took {}ms", t0.elapsed().as_millis());
        eprintln!("total time: {}ms", start_time.elapsed().as_millis());

        eprintln!("build time trace by kind:");
        eprintln!("{}", tracer.print_by_kind(interner));

        eprintln!("build time trace top entries:");
        eprintln!("{}", tracer.print_by_timing(interner));
    } else {
        eprintln!("total time: {}ms", start_time.elapsed().as_millis());
    }
}

#[tokio::main]
async fn main() {
    run().await;
}
