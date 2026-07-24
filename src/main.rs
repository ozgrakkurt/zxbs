use zxbs::{build, config, interner, plan, scan};

type Executor = smol::Executor<'static>;

async fn run(executor: &'static Executor) {
    let config = config::Config::parse_args();

    let _ = smol::fs::remove_dir_all(config.build_path).await;
    smol::fs::create_dir_all(config.build_path).await.unwrap();

    let mut interner = interner::Interner::new();
    let info = scan::scan(&config, &mut interner).await;

    let tasks = plan::plan(&config, &mut interner, &info);

    let build = Build::new(executor, tasks);

    build.await;
}

fn main() {
    let executor = Box::leak(Box::new(Executor::new()));
    smol::future::block_on(executor.run(run(executor)));
}
