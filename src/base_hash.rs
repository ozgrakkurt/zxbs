use crate::config::Config;
use crate::hash::{Hash, UnorderedHashCombiner, hash_bytes, hash_ordered};
use crate::thread_pool::{JoinSet, ThreadPool};

// YES, we have to do this multithreaded because running `clang --version` takes 8ms on my computer XDXDXD.
fn spawn_read_version(
    bin_name: &'static str,
    thread_pool: &ThreadPool,
    join_set: &mut JoinSet<Hash>,
) {
    join_set.spawn(thread_pool, move || {
        let output = match std::process::Command::new(bin_name)
            .arg("--version")
            .output()
        {
            Ok(out) => out,
            Err(e) => {
                panic!("Failed to run `{} --version`\nError = {:?}\n", bin_name, e,);
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8(output.stderr).unwrap();
            panic!(
                "Failed to run `{} --version`.\nStatus: {}\nStdErr:\n{}\n",
                bin_name, output.status, stderr,
            );
        }

        hash_bytes(&output.stdout)
    });
}

pub fn calculate_base_hash(config: &'static Config, thread_pool: &ThreadPool) -> Hash {
    let mut join_set = JoinSet::<Hash>::new();

    join_set.spawn(thread_pool, || {
        const SELF_EXE_PATH: &str = "/proc/self/exe";
        let self_exe_bytes = std::fs::read(SELF_EXE_PATH).unwrap();
        let self_exe_hash = hash_ordered(&[SELF_EXE_PATH.as_bytes(), self_exe_bytes.as_slice()]);
        self_exe_hash
    });

    for bin_name in [config.cxx, config.ar, config.clang_scan_deps] {
        spawn_read_version(bin_name, thread_pool, &mut join_set);
    }

    let mut combiner = UnorderedHashCombiner::new();

    while let Some(hash) = join_set.join_next() {
        combiner.add(hash);
    }

    combiner.combine()
}
