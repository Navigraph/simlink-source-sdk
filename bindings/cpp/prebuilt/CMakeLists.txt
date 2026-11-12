# The release archive's CMakeLists.txt: compiles the C++ half of the bindings in the user's build, against the
# prebuilt Rust library from `cargo xtask package cpp`.
cmake_minimum_required(VERSION 3.20)
project(simlink_source_sdk_cpp LANGUAGES CXX)

if(NOT CMAKE_SIZEOF_VOID_P EQUAL 8)
  message(FATAL_ERROR "The Simlink source SDK is 64-bit only")
endif()

add_library(simlink_source_sdk STATIC "${CMAKE_CURRENT_LIST_DIR}/src/simlink_source_sdk.cpp")
add_library(simlink-source-sdk::simlink-source-sdk ALIAS simlink_source_sdk)
target_compile_features(simlink_source_sdk PUBLIC cxx_std_20)
target_include_directories(simlink_source_sdk PUBLIC "${CMAKE_CURRENT_LIST_DIR}/include")
target_link_libraries(simlink_source_sdk PUBLIC
  "${CMAKE_CURRENT_LIST_DIR}/lib/${CMAKE_STATIC_LIBRARY_PREFIX}simlink_source_sdk${CMAKE_STATIC_LIBRARY_SUFFIX}")
# Rust's standard library imports from these, which MSVC doesn't link by default.
if(WIN32)
  target_link_libraries(simlink_source_sdk PUBLIC ntdll userenv)
endif()
