cmake_minimum_required(VERSION 3.24)
project(pipewright_ui
    VERSION 1.0.0
    DESCRIPTION "Pipewright Qt6 frontend"
    LANGUAGES C CXX
)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTORCC ON)
set(CMAKE_INCLUDE_CURRENT_DIR ON)

# --- Qt6 -------------------------------------------------------------
# Brew on macOS: prepend its Qt prefix to CMAKE_PREFIX_PATH automatically.
if(APPLE)
    execute_process(COMMAND brew --prefix qt
        OUTPUT_VARIABLE _BREW_QT_PREFIX
        OUTPUT_STRIP_TRAILING_WHITESPACE
        ERROR_QUIET
    )
    if(_BREW_QT_PREFIX)
        list(PREPEND CMAKE_PREFIX_PATH "${_BREW_QT_PREFIX}")
    endif()
endif()

find_package(Qt6 6.5 REQUIRED COMPONENTS Core Gui Quick Qml QuickControls2 Svg)
qt_standard_project_setup(REQUIRES 6.5)

# --- Corrosion (Rust ↔ CMake bridge) ---------------------------------
# Vendored via FetchContent so users only need `cmake` + Qt + cargo.
include(FetchContent)
FetchContent_Declare(
    Corrosion
    GIT_REPOSITORY https://github.com/corrosion-rs/corrosion.git
    GIT_TAG        v0.5.0
)
FetchContent_MakeAvailable(Corrosion)

# Build the cdylib from the parent workspace; Corrosion picks up the
# crate name and the resulting target is `pipeline_ffi-shared`.
corrosion_import_crate(
    MANIFEST_PATH ${CMAKE_CURRENT_SOURCE_DIR}/../../crates/pipeline-ffi/Cargo.toml
    CRATE_TYPES cdylib
)

# Generated header lives next to the Cargo source; expose it as an
# include dir on the imported target so consumers just write
# `#include <pipeline_ffi.h>`.
target_include_directories(pipeline_ffi INTERFACE
    ${CMAKE_CURRENT_SOURCE_DIR}/../../crates/pipeline-ffi/include
)

# --- The Qt application ----------------------------------------------
qt_add_executable(pipewright-ui
    src/main.cpp
    src/BridgeApi.h
    src/BridgeApi.cpp
    src/ProcessRunner.h
    src/ProcessRunner.cpp
    src/SyntaxHighlighter.h
    src/SyntaxHighlighter.cpp
)

# `src/` so the QML type-registration unit can find `BridgeApi.h`.
target_include_directories(pipewright-ui PRIVATE src)

qt_add_qml_module(pipewright-ui
    URI Pipewright
    VERSION 1.0
    QML_FILES
        src/qml/Main.qml
        src/qml/GraphEditor.qml
        src/qml/JobNode.qml
        src/qml/EditableRow.qml
        src/qml/CapabilitiesView.qml
        src/qml/MigrationView.qml
        src/qml/RecipesView.qml
        src/qml/RunView.qml
        src/qml/HumanView.qml
        src/qml/SettingsDialog.qml
    SOURCES
        src/BridgeApi.h
        src/BridgeApi.cpp
        src/ProcessRunner.h
        src/ProcessRunner.cpp
)

# UI-chrome translations. The .qm is compiled from i18n/pipewright_<locale>.ts
# (regenerate with: lupdate src/qml/*.qml src/*.cpp -ts i18n/pipewright_de.ts
# then lrelease). Embedded at :/i18n/<file> so BridgeApi::setLanguage can load
# it at runtime and retranslate the live QML.
qt_add_resources(pipewright-ui "translations"
    PREFIX "/i18n"
    BASE "${CMAKE_CURRENT_SOURCE_DIR}/i18n"
    FILES i18n/pipewright_de.qm
)

target_link_libraries(pipewright-ui PRIVATE
    Qt6::Core
    Qt6::Gui
    Qt6::Quick
    Qt6::Qml
    Qt6::QuickControls2
    Qt6::Svg
    pipeline_ffi
)

# macOS bundle metadata
if(APPLE)
    set_target_properties(pipewright-ui PROPERTIES
        MACOSX_BUNDLE TRUE
        MACOSX_BUNDLE_BUNDLE_NAME "Pipewright"
        MACOSX_BUNDLE_GUI_IDENTIFIER "org.zero-principle.pipewright"
        MACOSX_BUNDLE_BUNDLE_VERSION ${PROJECT_VERSION}
        MACOSX_BUNDLE_SHORT_VERSION_STRING ${PROJECT_VERSION}
    )
endif()

# --- Installer (CPack) ----------------------------------------------
# Per-OS installer generators:
#   macOS → .dmg via productbuild + create-dmg (CI signs/notarises)
#   Linux → .deb (Debian/Ubuntu) and .rpm (Fedora/RHEL)
#   Win   → .msi via WIX Toolset (CI signs with signtool)
include(InstallRequiredSystemLibraries)
install(TARGETS pipewright-ui
    BUNDLE  DESTINATION .          COMPONENT app
    RUNTIME DESTINATION bin        COMPONENT app
)

set(CPACK_PACKAGE_NAME              "pipewright")
set(CPACK_PACKAGE_VENDOR            "Zero Objects")
set(CPACK_PACKAGE_VERSION           "${PROJECT_VERSION}")
set(CPACK_PACKAGE_DESCRIPTION_SUMMARY
    "CI/CD pipeline comprehension and migration tool")
set(CPACK_PACKAGE_CONTACT           "mail@zero-principle.org")
set(CPACK_RESOURCE_FILE_LICENSE     "${CMAKE_SOURCE_DIR}/../../LICENSE")
set(CPACK_PACKAGE_FILE_NAME
    "pipewright-${PROJECT_VERSION}-${CMAKE_SYSTEM_NAME}-${CMAKE_SYSTEM_PROCESSOR}")

if(APPLE)
    set(CPACK_GENERATOR "DragNDrop")
    set(CPACK_DMG_VOLUME_NAME "Pipewright ${PROJECT_VERSION}")
elseif(WIN32)
    set(CPACK_GENERATOR "WIX")
    set(CPACK_WIX_UPGRADE_GUID "B23F4B0F-1B0E-4F2A-9B5D-4A8B0F1E2D3C")
elseif(UNIX)
    set(CPACK_GENERATOR "DEB;RPM")
    set(CPACK_DEBIAN_PACKAGE_DEPENDS "libqt6core6, libqt6gui6, libqt6quickcontrols2-6, libqt6svg6")
    set(CPACK_RPM_PACKAGE_REQUIRES "qt6-qtbase >= 6.5, qt6-qtdeclarative >= 6.5, qt6-qtsvg >= 6.5")
endif()
include(CPack)
