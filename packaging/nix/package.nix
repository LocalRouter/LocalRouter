# LocalRouter for Nix, repackaged from the released Linux .deb.
#
# Like the Flatpak, Snap and AUR recipes, this unpacks the published .deb
# instead of building from source: a Tauri build needs node plus a full npm
# install, which the Nix sandbox cannot do without vendoring the npm tree.
#
# The version and per-architecture hashes live in ./sources.json, which the
# `update-nix` job in .github/workflows/release.yml rewrites on every release
# (rendered by scripts/publish-packages.sh from ./sources.json.tmpl).
{
  lib,
  stdenv,
  fetchurl,
  dpkg,
  autoPatchelfHook,
  wrapGAppsHook3,
  glib,
  glib-networking,
  gtk3,
  cairo,
  gdk-pixbuf,
  pango,
  webkitgtk_4_1,
  libsoup_3,
  openssl,
  libayatana-appindicator,
  gst_all_1,
}:

let
  sources = lib.importJSON ./sources.json;

  # Nix system -> Debian architecture in the release asset name.
  debArch = {
    x86_64-linux = "amd64";
    aarch64-linux = "arm64";
  };

  system = stdenv.hostPlatform.system;
  arch = debArch.${system} or (throw "localrouter: unsupported system ${system}");
in
stdenv.mkDerivation {
  pname = "localrouter";
  inherit (sources) version;

  src = fetchurl {
    url = "https://github.com/LocalRouter/LocalRouter/releases/download/v${sources.version}/LocalRouter_${sources.version}_${arch}.deb";
    sha256 = sources.sha256.${system};
  };

  nativeBuildInputs = [
    dpkg
    autoPatchelfHook
    wrapGAppsHook3
  ];

  buildInputs = [
    stdenv.cc.cc.lib # libgcc_s
    glib
    # TLS for WebKit's own network stack (GIO_EXTRA_MODULES via the wrapper).
    glib-networking
    gtk3
    cairo
    gdk-pixbuf
    pango
    webkitgtk_4_1
    libsoup_3
    openssl
    # WebKit plays and records audio through GStreamer (the Try-it-out speech
    # panel); the wrapper exports these on GST_PLUGIN_SYSTEM_PATH_1_0.
    gst_all_1.gst-plugins-base
    gst_all_1.gst-plugins-good
  ];

  unpackPhase = ''
    runHook preUnpack
    dpkg-deb -x "$src" .
    runHook postUnpack
  '';

  dontConfigure = true;
  dontBuild = true;

  installPhase = ''
    runHook preInstall

    install -Dm755 usr/bin/localrouter "$out/bin/localrouter"

    # Tauri bundle resources (tauri.conf.json `bundle.resources`).
    if [ -d usr/lib/LocalRouter ]; then
      mkdir -p "$out/lib"
      cp -r usr/lib/LocalRouter "$out/lib/LocalRouter"
    fi

    install -Dm644 usr/share/applications/LocalRouter.desktop \
      "$out/share/applications/localrouter.desktop"
    cp -r usr/share/icons "$out/share/icons"

    # The deb's usr/share/localrouter/install-source marker says "deb" and is
    # only ever read from the absolute /usr/share path, so it is dropped here;
    # the wrapper's LOCALROUTER_INSTALL_SOURCE identifies this install instead.

    runHook postInstall
  '';

  preFixup = ''
    gappsWrapperArgs+=(
      # The tray icon backend dlopen()s libayatana-appindicator3.so.1 at
      # runtime, so it never shows up as a NEEDED entry for autoPatchelf.
      --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath [ libayatana-appindicator ]}
      # Tells crates/lr-utils/src/install_source.rs that Nix owns this
      # install, so the in-app updater stands down (the store is read-only).
      --set-default LOCALROUTER_INSTALL_SOURCE nix
    )
  '';

  meta = {
    description = "Local OpenAI-compatible API gateway with intelligent multi-provider routing";
    homepage = "https://localrouter.ai";
    downloadPage = "https://github.com/LocalRouter/LocalRouter/releases";
    changelog = "https://github.com/LocalRouter/LocalRouter/releases/tag/v${sources.version}";
    license = lib.licenses.agpl3Plus;
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
    platforms = builtins.attrNames debArch;
    mainProgram = "localrouter";
  };
}
