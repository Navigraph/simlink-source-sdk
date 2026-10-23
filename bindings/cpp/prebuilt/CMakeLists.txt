# The release archive's CMakeLists.txt: the prebuilt library from `cargo xtask package cpp`.
cmake_minimum_required(VERSION 3.20)
project(simlink_cpp LANGUAGES CXX)

if(NOT CMAKE_SIZEOF_VOID_P EQUAL 8)
  message(FATAL_ERROR "The Simlink source SDK is 64-bit only")
endif()

include("${CMAKE_CURRENT_LIST_DIR}/cmake/system-libraries.cmake")

add_library(simlink STATIC IMPORTED GLOBAL)
add_library(simlink::simlink ALIAS simlink)
target_include_directories(simlink INTERFACE "${CMAKE_CURRENT_LIST_DIR}/include")
target_compile_features(simlink INTERFACE cxx_std_20)
target_link_libraries(simlink INTERFACE ${SIMLINK_SYSTEM_LIBRARIES})

if(WIN32)
  # MSVC won't link Release C++ code into a Debug build.
  set_target_properties(simlink PROPERTIES
    IMPORTED_LOCATION "${CMAKE_CURRENT_LIST_DIR}/lib/Release/simlink.lib"
    IMPORTED_LOCATION_DEBUG "${CMAKE_CURRENT_LIST_DIR}/lib/Debug/simlink.lib")
else()
  set_target_properties(simlink PROPERTIES
    IMPORTED_LOCATION "${CMAKE_CURRENT_LIST_DIR}/lib/libsimlink.a")
endif()
