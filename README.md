# EDA Tools

This repository houses a common build infrastructure for EDA tools. The goal is
to provide a common set of tools and scripts to build and install EDA tools in a
consistent way and to share common build infrastructure.

Head over to the [releases section](https://github.com/axelera-ai/tools.hw.eda/releases) to find the most up-to-date releases.

## Supported Tools

- [`slang`](https://github.com/MikePopoloski/slang): A SystemVerilog parser and elaborator.
- [`verilator`](https://github.com/verilator/verilator): A fast SystemVerilog simulator that translates SystemVerilog to C++/SystemC.
- [`yosys`](https://github.com/YosysHQ/yosys): An open-source synthesiser, built with [`yosys-slang`](https://github.com/povik/yosys-slang) plugin for SystemVerilog support.

## Supported Platforms

- Linux (manylinux_2_28 / glibc 2.28+, ARM64 and x86_64)
- macOS (ARM64) - incl. code signing and notarization.

## Internals

Tools are added as git submodules. [Dependabot](https://docs.github.com/en/code-security/dependabot)
checks weekly for upstream updates and opens pull requests towards main. When a
submodule changes, CI builds only the affected tool(s) to verify the update.

Releases are manual and triggered via the GitHub [Actions UI](https://github.com/axelera-ai/tools.hw.eda/actions/workflows/build.yml).
On release, all tools are rebuilt and uploaded as platform-specific tarballs.

### Build details

- **Linux**: builds run inside [manylinux_2_28](https://github.com/pypa/manylinux)
  containers (AlmaLinux 8, glibc 2.28) for broad compatibility. Yosys is built
  with TCL, readline, and plugin support disabled to avoid dynamic library
  dependencies.
- **macOS**: binaries are code-signed with a Developer ID certificate and
  notarized via Apple's notary service.
