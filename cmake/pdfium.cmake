include_guard(GLOBAL)

set(PDFIUM_RELEASE "latest" CACHE STRING "pdfium-binaries release tag, or 'latest'")

bare_platform(_sys)
bare_arch(_arch)

if(_sys STREQUAL "darwin")
  set(_pdfium_asset "pdfium-mac-${_arch}")
elseif(_sys STREQUAL "linux")
  set(_pdfium_asset "pdfium-linux-${_arch}")
elseif(_sys STREQUAL "android")
  set(_pdfium_asset "pdfium-android-${_arch}")
elseif(_sys STREQUAL "win32")
  set(_pdfium_asset "pdfium-win-${_arch}")
elseif(_sys STREQUAL "ios")
  if(CMAKE_OSX_SYSROOT MATCHES "[Ss]imulator")
    set(_pdfium_asset "pdfium-ios-simulator-${_arch}")
  else()
    set(_pdfium_asset "pdfium-ios-device-${_arch}")
  endif()
else()
  message(FATAL_ERROR "simple-converter: no PDFium asset for '${_sys}/${_arch}'")
endif()

if(PDFIUM_RELEASE STREQUAL "latest")
  set(_pdfium_url "https://github.com/bblanchon/pdfium-binaries/releases/latest/download/${_pdfium_asset}.tgz")
else()
  set(_pdfium_url "https://github.com/bblanchon/pdfium-binaries/releases/download/${PDFIUM_RELEASE}/${_pdfium_asset}.tgz")
endif()

set(_pdfium_dir "${CMAKE_BINARY_DIR}/pdfium")

if(NOT EXISTS "${_pdfium_dir}/include/fpdfview.h")
  message(STATUS "simple-converter: fetching ${_pdfium_url}")
  file(DOWNLOAD "${_pdfium_url}" "${CMAKE_BINARY_DIR}/pdfium.tgz" STATUS _pdfium_status)
  list(GET _pdfium_status 0 _pdfium_code)
  if(NOT _pdfium_code EQUAL 0)
    message(FATAL_ERROR "simple-converter: PDFium download failed (${_pdfium_status}) from ${_pdfium_url}")
  endif()
  file(MAKE_DIRECTORY "${_pdfium_dir}")
  file(ARCHIVE_EXTRACT INPUT "${CMAKE_BINARY_DIR}/pdfium.tgz" DESTINATION "${_pdfium_dir}")
endif()

file(GLOB _pdfium_lib
  "${_pdfium_dir}/lib/libpdfium.dylib"
  "${_pdfium_dir}/lib/libpdfium.so"
  "${_pdfium_dir}/bin/pdfium.dll"
  "${_pdfium_dir}/lib/pdfium.dll"
)
list(GET _pdfium_lib 0 _pdfium_lib)
if(NOT _pdfium_lib)
  message(FATAL_ERROR "simple-converter: no PDFium shared library found under ${_pdfium_dir}")
endif()

if(APPLE)
  execute_process(COMMAND install_name_tool -id @rpath/libpdfium.dylib "${_pdfium_lib}")
endif()

add_library(pdfium SHARED IMPORTED GLOBAL)
set_target_properties(
  pdfium
  PROPERTIES
    IMPORTED_LOCATION "${_pdfium_lib}"
    IMPORTED_NO_SONAME TRUE
    INTERFACE_INCLUDE_DIRECTORIES "${_pdfium_dir}/include"
)

if(WIN32)
  file(GLOB _pdfium_implib "${_pdfium_dir}/lib/pdfium.dll.lib" "${_pdfium_dir}/lib/pdfium.lib")
  list(GET _pdfium_implib 0 _pdfium_implib)
  if(_pdfium_implib)
    set_target_properties(pdfium PROPERTIES IMPORTED_IMPLIB "${_pdfium_implib}")
  endif()
endif()
