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

- Linux (manylinux_2_28 / glibc 2.28+, Arm and X86)
- MacOS (latest) - incl. code signing and notarization.

## Internals

The tools are added as submodules and there is a weekly dependabot job that
checks for updates and opens a pull request twoards main. If there is a tool
update available the corresponding CI runs to check for any failing builds.

Releases are currently manual and need to be triggered from the Github [Actions
UI](https://github.com/axelera-ai/tools.hw.eda/actions/workflows/build.yml). On
release builds all tools are always rebuild.
