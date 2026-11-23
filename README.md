# zxbs

This is a build system for C++ projects that use C++20 modules.

It is only intended to work on linux x86_64 and it is assumed the underlying compiler tools are always clang/llvm.

Doesn't support depending on external code.

It is recommended to get familiar with [how C++ modules work in clang](https://clang.llvm.org/docs/StandardCPlusPlusModules.html) to understand what C++ modules are.

This document is intended to be read from start to finish in order to be understood and be helpful in using or modifying this project. It is discouraged to use this project
without reading and roughly understanding this document to the end.

## Reason

The main goal of this project to implement a build system for a very particular style of C++. I don't expect this to be useful to many people
as it is missing a lot of features that people would want.

The reason for publishing this project publicly is to just show my work and also I hope other people that want to implement something like this will find it
worthwhile to explore and helpful for their learning.

This build system was built for [zx](https://github.com/ozgrakkurt/zx). There are no other users of this build system other than `zx`.

## Goals

- Build shoud be as fast as possible with minimal overhead over the underlying clang calls and maximum parallelism.
- Cache should be sufficient so re-compilation and re-testing is always practically instant even when clang is slow or when we are running slow tests like fuzz tests.
- Caching and every other piece of the system should be as reliable and as well-tested as possible. User should never have to debug the build system.
- Implicit directory structure so the C++ developer doesn't have to list every file they want to include in some other build system file.
- Minimal number of dependencies.
- Everything is built-in. For example user shouldn't need to implement clang-format using makefiles etc. they can just use `zxbs fmt`.
- Features to make integrating fuzz tests easy.
- Configuration options are minimal so we can make sure every option works with other options and there are no edge cases.
- Built-in profiling tooling so it is very easy to visualize what is taking time in the build. This is crucial for both the user and the development of this tool.
- Don't support unneeded things. There is no header (.h) support at all since we are building around modules.

## Install

Go into the project root and run:

```
RUSTFLAGS="-C target-cpu=native" cargo install --path . --locked
```

## Usage

### CLI

The form of the command is:

*The flags CAN'T come after the subcommand or the subcommand-args, everything has to be in this order.*

```
zxbs [flags] <subcommand> [subcommand-args]
```

Available flags are:
- `--cxx=<clang_command_name>` defaults to `clang++`.
- `--ar=<archiver>` defaults to `llvm-ar`.
- `--clang-scan-deps=<clang_scan_deps_command_name>` defaults to `clang-scan-deps`.
- `--clang-format=<clang_format_command_name>` defaults to `clang-format`.
- `--opt-level=<level>`. This is directly passed to the c++ compiler via the `-O` flag. Defaults to `0`.
- `--[no]-dev-mode`. Enables or disables the `-DZX_DEV` definition passed into the c++ code. Defaults to enabled.
- `--[no]-debug-info`. Enables or disables debug info generation via c++ compiler flags like `-g`. Defaults to enabled.
- `--[no]-fuzz`. Enables or disables compiler instrumentation for fuzzing and also controls the `-DZX_FUZZ` define passed into the c++ code. Defaults to disabled.
- `--[no]-fuzz-unlimited`. Enables or disables the `-DZX_FUZZ_UNLIMITED` define passed into c++ code. This is intended to control whether if fuzz tests will run indefinitely
      or they will terminate after some time. Defaults to disabled.
- `--src-dir=<path>`. Path to the source directory. Defaults to `src`.
- `--build-dir=<path>`. Path to output build artifacts including the cache. Defaults to `build`.
- `--[no]-time-trace`. Enables or disables the `chrome://tracing` formatted timing output generation. Disabled by default.
- `--[no]-cxx-time-trace`. Enables or disables passing `-ftime-trace` flag to clang which makes it generate `chrome://tracing` formatted files per compilation unit. These
  files can later be found in the build directory.
- `--[no]-asm`. Enables or disables generating annotated assembly with `-S -fverbose-asm` next to every library, binary and test object file.
  Module object files are not emitted. The `.s` files are treated like any other build artifact and are stored in the cache. Disabled by default.

Available subcommands are:
- `run <entry_name:binary_name> [target_arg]*` -> Builds everything and then runs the indicated binary target. The [target_arg]* list is passed through to the binary that is being run.
- `test [entry_name:test_name]` -> Builds everything and then runs the indicated test or runs all tests if there is no argument.
- `build` -> Just builds everything.
- `fmt` -> Formats all of the source files using clang-format.
- `clean` -> Deletes the cache.

### Directory Structure

The C++ project is expected to have a particular directory structure.

The base src and build directories are configurable via CLI flags but the rest of the
structure including the file extensions are fixed.

The project consists of a number of entries.

Each entry has a module, library, binary and test part. And each entry folder has a `deps` file that is a regular text file that contains
a list of the names of the dependencies of this entry, each on a seperate line.

The module part consists of `.cppm` (C++20 module) files. They implement the interfacing between the entries.
They are expected to also contain some of implementation as (unlike the library part) they can be inlined across entries for optimization by the compiler.
Each entry is expected to at least have a module file named `<entry_name>.cppm`, this is the root module file of the entry. Each root module file is expected to
depend on all the other module files in the entry.

The library part is the regular `.cc` files you would expect in a c++ project. They just output object files and then get linked into a static library that gets
linked into any executable that ends up depending on this entry.

The binary part contains one `.cc` file per binary target. So each `.cc` file that is in the `bin` folder will generate an executable in the output.

The test part also contains only `.cc` files. These `.cc` files are expected to be regular c++ programs that just return non-zero status codes and print error messages to stderr if a test fails.

Binaries, tests and library files implicitly depend on their parent entry and whatever entries that their parent entry depends on. They can only import root modules of their parent entry or root modules
of any other entry that their parent entry depends on.

All source files are scanned and found automatically. File extensions (.cppm/.cc) and expected locations of the source files are hardcoded into the implementation and can't be changed via configuration.

- src/
  - <entry_name>
    - bin
      - <executable_name>.cc
    - test
      - <test_name>.cc
    - lib
      - <library_implementation_file>.cc
    - mod
      - <module_file_name>.cppm
    - deps

- build/ (output)
  - cache
    - ..
  - out
    - mod
      - <some_module_output>.pcm (these come from building the cppm files.)
    - entry
      - <entry_name>
        - lib
          - object files
        - bin
          - object files
          - executables
        - test
          - test executables
          - stamp
            - test stamps that are emitted if a test runs successfully, used for cache purposes.

When `--asm` is enabled, an annotated `.s` file is emitted next to every library, binary and test object file in the output tree.

### Profiling

Enabling the `--time-trace` flag makes the build system output a `time_trace.json` in the `build/out` directory.

This file can be opened in a chromium browser by going to the `chrome://tracing` page and opening it.

It can also be opened in `Perfetto`.

This is crucial for keeping your project building fast. It is recommended to check the profiles as soon as you find the total
build time getting longer and it is also recommended to check the uncached full build time of your c++ project from time to time using
something like this:

```
zxbs clean
time zxbs --time-trace test
```

## Development (of `zxbs` project)

### Test

Regular tests:

`cargo test --all-features --all-targets`

Fuzz tests:

`make fuzz`

### Lint - Format

`cargo clippy --all-targets --all-features`

`cargo fmt`

Go into the `fuzz` directory and repeat the commands with `cargo +nightly`.
