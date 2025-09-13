%global crate motoc

Name:           %{crate}
Version:        0.3.4
Release:        1%{?dist}
Summary:        Monado Tracking Origin Calibrator

License:        MIT
URL:            https://github.com/galister/motoc
Source0:        %{url}/archive/v%{version}/%{name}-%{version}.tar.gz

# GUI and build dependencies
BuildRequires:  rust
BuildRequires:  cargo
BuildRequires:  gcc
BuildRequires:  libX11-devel
BuildRequires:  libxcb-devel
BuildRequires:  libXcursor-devel
BuildRequires:  libXrandr-devel
BuildRequires:  libXi-devel
BuildRequires:  libxkbcommon-devel
BuildRequires:  wayland-devel
BuildRequires:  wayland-protocols-devel
BuildRequires:  mesa-libGL-devel
BuildRequires:  vulkan-headers

# VR dependencies (likely from Terra or another third-party repo)
BuildRequires:  openxr-devel
BuildRequires:  monado-devel

# Runtime dependencies
Requires:       openxr
Requires:       monado

%description
A tool that allows users to calibrate devices of different tracking origins
(tracking technologies) to work together, for example a Quest HMD with Vive trackers.
This package includes both a command-line tool and a GUI.

%prep
%autosetup -n %{name}-%{version}

%build
# The crate is both a library and a binary, so we build the binary specifically.
cargo build --release --bin %{crate}

%install
# Install the binary
install -Dpm 0755 target/release/%{crate} %{buildroot}%{_bindir}/%{crate}

# Install the .desktop file and icon
# Note: We are not creating an icon file here, just referencing it.
# The user or another package would need to provide /usr/share/icons/hicolor/256x256/apps/motoc.png
install -Dpm 0644 motoc.desktop %{buildroot}%{_datadir}/applications/%{crate}.desktop
# Create a dummy icon path for the spec to own, though the file won't exist in this build.
install -d %{buildroot}%{_datadir}/icons/hicolor/256x256/apps/
touch %{buildroot}%{_datadir}/icons/hicolor/256x256/apps/%{crate}.png


%files
%license LICENSE
%doc README.md
%{_bindir}/%{crate}
%{_datadir}/applications/%{crate}.desktop
%{_datadir}/icons/hicolor/256x256/apps/%{crate}.png

%changelog
* Sun May 12 2024 Jules <jules@agent.dev> - 0.3.4-1
- Initial RPM packaging with GUI support.
