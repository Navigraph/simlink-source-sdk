cmake_minimum_required(VERSION 3.20)
project(simlink_source_sdk_cpp LANGUAGES CXX)

set(SIMLINK_SOURCE_SDK_LIBRARY_DIR "${CMAKE_CURRENT_LIST_DIR}/../../target/release"
    CACHE PATH "Directory holding the library from `cargo xtask build`")
set(SIMLINK_SOURCE_SDK_BINDINGS_DIR "${CMAKE_CURRENT_LIST_DIR}/../../target/bindings/cpp"
    CACHE PATH "Directory holding the C++ bindings from `cargo xtask build`")

if(NOT CMAKE_SIZEOF_VOID_P EQUAL 8)
  message(FATAL_ERROR "The Simlink source SDK is 64-bit only")
endif()

if(NOT EXISTS "${SIMLINK_SOURCE_SDK_BINDINGS_DIR}/simlink_source_sdk.cpp")
  message(FATAL_ERROR
    "No C++ bindings in ${SIMLINK_SOURCE_SDK_BINDINGS_DIR}; run `cargo xtask build` first")
endif()

add_library(simlink_source_sdk STATIC "${SIMLINK_SOURCE_SDK_BINDINGS_DIR}/simlink_source_sdk.cpp")
add_library(simlink-source-sdk::simlink-source-sdk ALIAS simlink_source_sdk)
target_compile_features(simlink_source_sdk PUBLIC cxx_std_20)
target_include_directories(simlink_source_sdk PUBLIC "${SIMLINK_SOURCE_SDK_BINDINGS_DIR}")

target_link_libraries(simlink_source_sdk PUBLIC
  "${SIMLINK_SOURCE_SDK_LIBRARY_DIR}/${CMAKE_STATIC_LIBRARY_PREFIX}simlink_source_sdk${CMAKE_STATIC_LIBRARY_SUFFIX}")
# Rust's standard library imports from these, which MSVC doesn't link by default.
if(WIN32)
  target_link_libraries(simlink_source_sdk PUBLIC ntdll userenv)
endif()
