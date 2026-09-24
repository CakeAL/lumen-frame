if(NOT VCPKG_TARGET_IS_WINDOWS OR NOT VCPKG_TARGET_ARCHITECTURE STREQUAL "x64")
    message(FATAL_ERROR "This local libvips package requires x64 Windows")
endif()

# The upstream development archive ships one release build of each binary.
set(VCPKG_POLICY_MISMATCHED_NUMBER_OF_BINARIES enabled)

set(VIPS_DEV_ROOT "C:/Users/CakeAL/Desktop/vsc/lumen-frame/vips-dev-8.18")
if(NOT EXISTS "${VIPS_DEV_ROOT}/lib/libvips.lib" OR NOT EXISTS "${VIPS_DEV_ROOT}/bin/libvips-42.dll")
    message(FATAL_ERROR "libvips 8.18.6 development files not found at ${VIPS_DEV_ROOT}")
endif()

file(COPY "${VIPS_DEV_ROOT}/include/" DESTINATION "${CURRENT_PACKAGES_DIR}/include")
file(MAKE_DIRECTORY "${CURRENT_PACKAGES_DIR}/lib" "${CURRENT_PACKAGES_DIR}/bin")
foreach(lib IN ITEMS libvips.lib libglib-2.0.lib libgobject-2.0.lib)
    file(COPY "${VIPS_DEV_ROOT}/lib/${lib}" DESTINATION "${CURRENT_PACKAGES_DIR}/lib")
endforeach()
file(GLOB runtime_dlls "${VIPS_DEV_ROOT}/bin/*.dll")
file(COPY ${runtime_dlls} DESTINATION "${CURRENT_PACKAGES_DIR}/bin")
vcpkg_install_copyright(FILE_LIST "${VIPS_DEV_ROOT}/LICENSE")
