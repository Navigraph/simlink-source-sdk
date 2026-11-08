# The release archive's CMakeLists.txt: the prebuilt library from `cargo xtask package cpp`.
cmake_minimum_required(VERSION 3.20)
project(simlink_source_sdk_cpp LANGUAGES CXX)

if(NOT CMAKE_SIZEOF_VOID_P EQUAL 8)
  message(FATAL_ERROR "The Simlink source SDK is 64-bit only")
endif()

add_library(simlink_source_sdk STATIC IMPORTED GLOBAL)
add_library(simlink-source-sdk::simlink-source-sdk ALIAS simlink_source_sdk)
target_include_directories(simlink_source_sdk INTERFACE "${CMAKE_CURRENT_LIST_DIR}/include")
target_compile_features(simlink_source_sdk INTERFACE cxx_std_20)

if(WIN32)
  # Rust's standard library imports from these, which MSVC doesn't link by default.
  target_link_libraries(simlink_source_sdk INTERFACE ntdll userenv)
  # MSVC won't link Release C++ code into a Debug build.
  set_target_properties(simlink_source_sdk PROPERTIES
    IMPORTED_LOCATION "${CMAKE_CURRENT_LIST_DIR}/lib/Release/simlink_source_sdk.lib"
    IMPORTED_LOCATION_DEBUG "${CMAKE_CURRENT_LIST_DIR}/lib/Debug/simlink_source_sdk.lib")
else()
  set_target_properties(simlink_source_sdk PROPERTIES
    IMPORTED_LOCATION "${CMAKE_CURRENT_LIST_DIR}/lib/libsimlink_source_sdk.a")
endif()
