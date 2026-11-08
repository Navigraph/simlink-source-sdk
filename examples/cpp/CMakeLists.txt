cmake_minimum_required(VERSION 3.20)
project(simlink_source_sdk_examples LANGUAGES CXX)

# Same folder for every generator, without a Release subfolder.
set(CMAKE_RUNTIME_OUTPUT_DIRECTORY_RELEASE "${CMAKE_BINARY_DIR}/bin")

add_subdirectory(../../bindings/cpp simlink-source-sdk)

foreach(example quickstart traffic commands diagnostics)
  add_executable(${example} ${example}.cpp)
  target_link_libraries(${example} PRIVATE simlink-source-sdk::simlink-source-sdk)
endforeach()
