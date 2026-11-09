{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, rust-overlay }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; overlays = [ rust-overlay.overlays.default ]; };
      # liest rust-toolchain.toml (stable + rustfmt + clippy)
      toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
      libs = with pkgs; [
        wayland libxkbcommon libGL vulkan-loader
        xorg.libX11 xorg.libXcursor xorg.libXi xorg.libXrandr xorg.libxcb
      ];
    in {
      devShells.${system}.default = pkgs.mkShell {
        nativeBuildInputs = [ toolchain pkgs.rust-analyzer pkgs.cargo-deny pkgs.pkg-config ];
        buildInputs = libs;
        # eframe lädt Wayland/X11/GL/Vulkan zur Laufzeit per dlopen
        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath libs;
      };
    };
}
