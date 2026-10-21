cmake_minimum_required(VERSION 3.20)
project(simlink_cpp LANGUAGES CXX)

set(SIMLINK_LIBRARY_DIR "${CMAKE_CURRENT_LIST_DIR}/../../target/release"
    CACHE PATH "Directory holding the library from `cargo xtask build`")
set(SIMLINK_BINDINGS_DIR "${CMAKE_CURRENT_LIST_DIR}/../../target/bindings/cpp"
    CACHE PATH "Directory holding the C++ bindings from `cargo xtask build`")

if(NOT CMAKE_SIZEOF_VOID_P EQUAL 8)
  message(FATAL_ERROR "The Simlink source SDK is 64-bit only")
endif()

if(NOT EXISTS "${SIMLINK_BINDINGS_DIR}/simlink.cpp")
  message(FATAL_ERROR
    "No C++ bindings in ${SIMLINK_BINDINGS_DIR}; run `cargo xtask build` first")
endif()

add_library(simlink STATIC "${SIMLINK_BINDINGS_DIR}/simlink.cpp")
add_library(simlink::simlink ALIAS simlink)
target_compile_features(simlink PUBLIC cxx_std_20)
target_include_directories(simlink PUBLIC "${SIMLINK_BINDINGS_DIR}")

# From `cargo rustc -- --print native-static-libs`.
if(WIN32)
  target_link_libraries(simlink PUBLIC "${SIMLINK_LIBRARY_DIR}/simlink.lib"
    kernel32 ntdll userenv ws2_32 dbghelp)
elseif(APPLE)
  target_link_libraries(simlink PUBLIC "${SIMLINK_LIBRARY_DIR}/libsimlink.a"
    iconv)
else()
  target_link_libraries(simlink PUBLIC "${SIMLINK_LIBRARY_DIR}/libsimlink.a"
    gcc_s util rt pthread m dl)
endif()
