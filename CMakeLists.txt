cmake_minimum_required(VERSION 4.0)

find_package(cmake-bare REQUIRED PATHS node_modules/cmake-bare)
find_package(cmake-fetch REQUIRED PATHS node_modules/cmake-fetch)

project(simple_converter C)

include(cmake/rust.cmake)

bare_target(target)

add_bare_module(simple_converter NAME bare-x-to-pdf-${target})

target_sources(
  ${simple_converter}
  PRIVATE
    binding.c
)

target_include_directories(
  ${simple_converter}
  PRIVATE
    core/include
)

target_link_libraries(
  ${simple_converter}
  PRIVATE
    simple_converter_core
)
