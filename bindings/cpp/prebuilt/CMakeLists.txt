# The release archive's CMakeLists.txt: compiles the C++ half of the bindings in the user's build, against the
# prebuilt Rust library from `cargo xtask package cpp`. Built on its own, it installs as the CMake package
# `simlink-source-sdk`.
cmake_minimum_required(VERSION 3.20)
# `cargo xtask package cpp` writes in the release's version.
project(simlink_source_sdk_cpp VERSION 0.0.0 LANGUAGES CXX)

if(NOT CMAKE_SIZEOF_VOID_P EQUAL 8)
  message(FATAL_ERROR "The Simlink source SDK is 64-bit only")
endif()

include(CMakePackageConfigHelpers)
include(GNUInstallDirs)

set(rust_library "${CMAKE_STATIC_LIBRARY_PREFIX}simlink_source_sdk${CMAKE_STATIC_LIBRARY_SUFFIX}")

add_library(simlink_source_sdk STATIC "${CMAKE_CURRENT_LIST_DIR}/src/simlink_source_sdk.cpp")
add_library(simlink-source-sdk::simlink-source-sdk ALIAS simlink_source_sdk)
# Renamed so it installs next to the Rust library, which has the target's name, and suffixed in Debug so both
# configurations install to one prefix.
set_target_properties(simlink_source_sdk PROPERTIES
  OUTPUT_NAME simlink_source_sdk_bindings
  DEBUG_POSTFIX d
  EXPORT_NAME simlink-source-sdk)
target_compile_features(simlink_source_sdk PUBLIC cxx_std_20)
target_include_directories(simlink_source_sdk PUBLIC
  "$<BUILD_INTERFACE:${CMAKE_CURRENT_LIST_DIR}/include>"
  "$<INSTALL_INTERFACE:${CMAKE_INSTALL_INCLUDEDIR}>")
target_link_libraries(simlink_source_sdk PUBLIC
  "$<BUILD_INTERFACE:${CMAKE_CURRENT_LIST_DIR}/lib/${rust_library}>"
  "$<INSTALL_INTERFACE:$<INSTALL_PREFIX>/${CMAKE_INSTALL_LIBDIR}/${rust_library}>")
# Rust's standard library imports from these, which MSVC doesn't link by default.
if(WIN32)
  target_link_libraries(simlink_source_sdk PUBLIC ntdll userenv)
# And from these, which glibc before 2.34 keeps out of libc.
elseif(UNIX AND NOT APPLE)
  target_link_libraries(simlink_source_sdk PUBLIC pthread dl m rt util)
endif()

# Added with add_subdirectory, the SDK stays out of the parent project's install.
if(CMAKE_CURRENT_SOURCE_DIR STREQUAL CMAKE_SOURCE_DIR)
  install(TARGETS simlink_source_sdk EXPORT simlink-source-sdk)
  install(DIRECTORY "${CMAKE_CURRENT_LIST_DIR}/include/" TYPE INCLUDE)
  install(FILES "${CMAKE_CURRENT_LIST_DIR}/lib/${rust_library}" TYPE LIB)
  install(EXPORT simlink-source-sdk
    NAMESPACE simlink-source-sdk::
    FILE simlink-source-sdk-config.cmake
    DESTINATION "${CMAKE_INSTALL_DATADIR}/simlink-source-sdk")
  write_basic_package_version_file(simlink-source-sdk-config-version.cmake COMPATIBILITY SameMajorVersion)
  install(FILES "${CMAKE_CURRENT_BINARY_DIR}/simlink-source-sdk-config-version.cmake"
    DESTINATION "${CMAKE_INSTALL_DATADIR}/simlink-source-sdk")
endif()
