# zxbs

This is a build system written in Rust, to compile C++ projects that use C++20 modules using clang.

It is only intended to work on linux x86_64 and produce code for linux x86_64.

Doesn't support depending on external code or even linking in libc. The C++ project is expected to implement memcpy/memcmp/_start etc. itself.

It is required to get familiar with [how C++ modules work in clang](https://clang.llvm.org/docs/StandardCPlusPlusModules.html) in order to understand this document.

This document is intended to be read from start to finish in order to be understood and be helpful in using or modifying this project. It is discouraged to use this project
without reading and roughly understanding this document to the end.

## Reason

The main goal of this project to implement a build system for a very particular style of C++. I don't expect this to be useful to many people
as it is missing a lot of features that people would want.

The reason for publishing this project publicly is to just show my work and also I hope other people that want to implement something like this will find it
worthwhile to explore and helpful for their learning.

The main reason I built this project is the frustration I had with cmake/ninja and python/ninja setups I had. I wanted to have a build system
that worked nicely with C++ modules, fuzz testing and didn't require me to list source files a second time in build files. I also wanted
it to be easy to modify for my usecase so I wouldn't need to learn cmake to bend it to for my usecase.

My usecase was mostly covered by using a python script that generated ninja files but that was slow and clunky and didn't seem to be possible to adapt
to include c++ modules support. I switched to rust for implementing the config generation and then I realized I don't need ninja at that point.

## Goals

- Never break the API of the system. If a C++ project compiles with this build system now, it should keep compiling forever. This is only for the CLI usage aspect, there is no such guarantee internally in this codebase.
- Build shoud be as fast as possible with minimal overhead over the underlying clang calls and maximum parallelism.
- Cache should be sufficient so re-compilation and re-testing is always practically instant even when clang is slow or when we are running slow tests like fuzz tests.
- Caching and every other piece of the system should be as reliable and as well-tested as possible. User should never have to debug the build system.
- Implicit directory structure so the C++ developer doesn't have to list every file they want to include in some other build system file.
- Minimal number of dependencies.
- Static linking with musl should work so it is possible to easily make a release binary that is portable between computers.
- Everything is built-in. For example user shouldn't need to implement clang-format using makefiles etc. they can just use `zxbs fmt`.
- Configuration options are minimal so we can make sure every option works with other options and there are no edge cases.
- Built-in profiling tooling so it is very easy to visualize what is taking time in the build. Crucial for both the user and the development of this tool.
- Don't support unneeded things. There is no header (.h) support at all since we are using modules for interfacing between parts of the project.

## Directory Structure

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

All source files are scanned and found automatically. Extensions and locations of the source files are hardcoded into the implementation and can't be changed via configuration.

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

## Install

Go into the project root and run:

```
RUSTFLAGS="-C target-cpu=native" cargo install --path . --locked
```

## CLI

The form of the command is:

*The flags CAN'T come after the subcommand or the subcommand-args, everything has to be in this order.*

```
zxbs [flags] <subcommand> [subcommand-args]
```

Available flags are:

Available subcommands are:

## Example projects / Projects that use this build system

- [zx]()

## LLM usage

LLM assistence is only reviews and prototyping on the implementation (src/) part of the project. Any code that LLM writes for implementation was completely re-written to make it reasonable.

LLMs generated code does exist in the test and fuzz tests parts but I also try to mostly review this part code to at least make sure it is hitting the wanted parts of the implementation in a rasonable amount of ways.

## Implementation

It is strongly recommended to go into the code at this point and at least understand how configuration and scanning work.

The implementation details and architecture are documented in the code.

## Contribution

I don't expect anyone to contribute code to this project. I instead encourage you to build what you want in your way and just take the bits of this project where you see fit.
