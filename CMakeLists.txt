cmake_minimum_required(VERSION 4.0)

find_package(cmake-bare REQUIRED PATHS node_modules/cmake-bare)
find_package(cmake-fetch REQUIRED PATHS node_modules/cmake-fetch)

project(simple_converter C)

include(cmake/rust.cmake)
include(cmake/pdfium.cmake)

add_bare_module(simple_converter INSTALL TARGET pdfium)

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
