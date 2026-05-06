# CMakeLists.txt — AeroACARS X-Plane Native Plugin
#
# Builds a single shared-library plugin (.xpl) for X-Plane 11/12 on
# Windows / macOS / Linux. Linked against the X-Plane Plugin SDK 4.3.0
# (vendored under third_party/XPSDK430/, BSD-licensed).
#
# Plugin output naming (X-Plane convention — fat folder layout):
#   AeroACARS/
#       64/
#           win.xpl          (Windows x86_64)
#           mac.xpl          (macOS universal, x86_64 + arm64)
#           lin.xpl          (Linux x86_64)
#
# Build flow per platform:
#   Windows:   cmake -B build -G "Visual Studio 17 2022" -A x64
#              cmake --build build --config Release --target AeroACARS
#   macOS:     cmake -B build -G "Unix Makefiles" \
#                    -DCMAKE_OSX_ARCHITECTURES="x86_64;arm64"
#              cmake --build build --config Release --target AeroACARS
#   Linux:     cmake -B build -G "Unix Makefiles"
#              cmake --build build --config Release --target AeroACARS
#
# Output ends up in `build/AeroACARS/64/<platform>.xpl` regardless of
# generator — the post-build step renames + relocates as needed.

cmake_minimum_required(VERSION 3.20)
project(AeroACARS_XPlane_Plugin
    VERSION 0.5.0
    LANGUAGES CXX
    DESCRIPTION "AeroACARS X-Plane Premium Plugin"
)

# ---- C++ standard ---------------------------------------------------------
# C++17 — modern enough for std::optional / structured bindings, conservative
# enough that the X-Plane SDK headers (which are C99) compile cleanly via
# `extern "C"`. Static runtime linking on Windows so the .xpl has no MSVC
# DLL dependencies the pilot has to install.
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

if(MSVC)
    set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
    add_compile_options(/W4 /permissive-)
    # /MP for parallel build, /Zc:__cplusplus to actually report C++17.
    add_compile_options(/MP /Zc:__cplusplus)
    # Stop windows.h from poisoning min/max macros.
    add_compile_definitions(NOMINMAX WIN32_LEAN_AND_MEAN _CRT_SECURE_NO_WARNINGS)
else()
    add_compile_options(-Wall -Wextra -Wpedantic -fvisibility=hidden)
    # No exceptions / RTTI — keeps the binary small and avoids edge cases
    # with X-Plane's C ABI plugin boundary. We program defensively in
    # plain C++ style (return codes, std::optional) instead.
    add_compile_options(-fno-exceptions -fno-rtti)
endif()

# ---- X-Plane SDK ----------------------------------------------------------
set(XPLM_SDK_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/third_party/XPSDK430")
if(NOT EXISTS "${XPLM_SDK_ROOT}/CHeaders/XPLM/XPLMDefs.h")
    message(FATAL_ERROR
        "X-Plane SDK not found at ${XPLM_SDK_ROOT}.\n"
        "Expected to find third_party/XPSDK430/CHeaders/XPLM/*.h.\n"
        "Re-clone the repo (the SDK is vendored) or run scripts/fetch_sdk.sh."
    )
endif()

# XPLM SDK feature defines — minimum versions we target.
# XPLM200 = X-Plane 9.0+ (legacy baseline, all our APIs available)
# XPLM210 = X-Plane 10.0+
# XPLM300 = X-Plane 11.0+
# XPLM301 = X-Plane 11.10+
# XPLM303 = X-Plane 11.50+ (Vulkan/Metal)
# We target XPLM303 = X-Plane 11.50+ minimum, which covers ~99% of pilots
# in 2026 and gives us access to all modern DataRef / flight-loop APIs.
add_compile_definitions(
    XPLM200=1
    XPLM210=1
    XPLM300=1
    XPLM301=1
    XPLM303=1
)

# Platform define expected by the SDK headers.
if(WIN32)
    add_compile_definitions(IBM=1 APL=0 LIN=0)
elseif(APPLE)
    add_compile_definitions(IBM=0 APL=1 LIN=0)
elseif(UNIX)
    add_compile_definitions(IBM=0 APL=0 LIN=1)
else()
    message(FATAL_ERROR "Unsupported platform")
endif()

# ---- Plugin target --------------------------------------------------------
add_library(AeroACARS MODULE
    src/plugin.cpp
)

target_include_directories(AeroACARS PRIVATE
    "${XPLM_SDK_ROOT}/CHeaders"
    "${XPLM_SDK_ROOT}/CHeaders/XPLM"
    "${XPLM_SDK_ROOT}/CHeaders/Widgets"
    src
)

# ---- Platform-specific linking --------------------------------------------
if(WIN32)
    # Link against X-Plane's import libraries.
    target_link_libraries(AeroACARS PRIVATE
        "${XPLM_SDK_ROOT}/Libraries/Win/XPLM_64.lib"
        ws2_32  # Winsock for our UDP socket
    )
    # X-Plane expects .xpl files, not .dll. Override suffix.
    set_target_properties(AeroACARS PROPERTIES
        SUFFIX ".xpl"
        OUTPUT_NAME "win"
    )
elseif(APPLE)
    # macOS link strategy — same approach every modern open-source X-Plane
    # plugin uses (xgs, X-RAAS, etc.). XPLM symbols are NOT resolved at
    # link time; the .xpl is shipped with undefined symbols and X-Plane
    # provides its own copy of XPLM at load time. This means:
    #   * No `-framework XPLM` (would force link-time resolution and tie
    #     us to a specific framework path embedded as a load command).
    #   * `-undefined dynamic_lookup` permits undefined symbols at link
    #     time (otherwise the linker rejects every XPLM* call).
    #   * `-flat_namespace` keeps the symbol-resolution model compatible
    #     with X-Plane's own loader.
    # The XPLM headers are still needed at compile time and come via the
    # `target_include_directories` block above.
    set_target_properties(AeroACARS PROPERTIES
        # Universal binary — both Apple Silicon and Intel.
        OSX_ARCHITECTURES "x86_64;arm64"
        # CMake auto-prepends "lib" to MODULE libs on Unix; X-Plane
        # expects exactly `mac.xpl` (no prefix).
        PREFIX ""
        SUFFIX ".xpl"
        OUTPUT_NAME "mac"
        # Plugin doesn't go in an .app bundle, RPATH is irrelevant.
        MACOSX_RPATH FALSE
    )
    target_link_options(AeroACARS PRIVATE
        "-undefined" "dynamic_lookup"
        "-Wl,-flat_namespace"
    )
elseif(UNIX)
    # Linux: no import library, no framework. X-Plane's main binary
    # provides every XPLM* symbol at load time; the .xpl just ships
    # with them undefined.
    #
    # CMake on some distros (notably Ubuntu 22.04+ with newer ld)
    # passes `--as-needed` and would warn about unused libraries —
    # we don't link any, so this is fine. We deliberately do NOT pass
    # `-Wl,--no-undefined` (which would error on every XPLM* call).
    set_target_properties(AeroACARS PROPERTIES
        # CMake auto-prepends "lib" to MODULE libs on Unix; X-Plane
        # expects exactly `lin.xpl` (no prefix).
        PREFIX ""
        SUFFIX ".xpl"
        OUTPUT_NAME "lin"
    )
endif()

# ---- Output layout (X-Plane fat-plugin folder convention) -----------------
# X-Plane expects:
#   <X-Plane>/Resources/plugins/AeroACARS/64/{win,mac,lin}.xpl
# We produce that exact layout in build/AeroACARS/64/ so packaging is a
# straight folder-copy.
set_target_properties(AeroACARS PROPERTIES
    LIBRARY_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/AeroACARS/64"
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/AeroACARS/64"
    LIBRARY_OUTPUT_DIRECTORY_DEBUG "${CMAKE_BINARY_DIR}/AeroACARS/64"
    LIBRARY_OUTPUT_DIRECTORY_RELEASE "${CMAKE_BINARY_DIR}/AeroACARS/64"
    RUNTIME_OUTPUT_DIRECTORY_DEBUG "${CMAKE_BINARY_DIR}/AeroACARS/64"
    RUNTIME_OUTPUT_DIRECTORY_RELEASE "${CMAKE_BINARY_DIR}/AeroACARS/64"
)

# Copy README and license files into the output folder so the packaged
# plugin contains attribution as required by the X-Plane SDK license.
configure_file(
    "${CMAKE_CURRENT_SOURCE_DIR}/README.md"
    "${CMAKE_BINARY_DIR}/AeroACARS/README.md"
    COPYONLY
)
configure_file(
    "${XPLM_SDK_ROOT}/license.txt"
    "${CMAKE_BINARY_DIR}/AeroACARS/XPLM_SDK_LICENSE.txt"
    COPYONLY
)

# ---- Diagnostic summary ---------------------------------------------------
message(STATUS "──────────────────────────────────────────────────────────")
message(STATUS " AeroACARS X-Plane Plugin v${PROJECT_VERSION}")
message(STATUS "  C++ Standard:  ${CMAKE_CXX_STANDARD}")
message(STATUS "  Target system: ${CMAKE_SYSTEM_NAME}")
message(STATUS "  Build type:    ${CMAKE_BUILD_TYPE}")
message(STATUS "  Output:        ${CMAKE_BINARY_DIR}/AeroACARS/64/")
message(STATUS "──────────────────────────────────────────────────────────")
