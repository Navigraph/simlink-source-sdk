cmake_minimum_required(VERSION 3.20)
project(simlink_examples LANGUAGES CXX)

# Same folder for every generator, without a Release subfolder.
set(CMAKE_RUNTIME_OUTPUT_DIRECTORY_RELEASE "${CMAKE_BINARY_DIR}/bin")

add_subdirectory(../../bindings/cpp simlink)

foreach(example quickstart commands diagnostics)
  add_executable(${example} ${example}.cpp)
  target_link_libraries(${example} PRIVATE simlink::simlink)
endforeach()
