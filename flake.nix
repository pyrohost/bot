{
  description = "Ember Discord Bot";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.rust-analyzer-src.follows = "";
    };
    flake-utils.url = "github:numtide/flake-utils";
    advisory-db = {
      url = "github:rustsec/advisory-db";
      flake = false;
    };
  };

  outputs = {
    self,
    nixpkgs,
    crane,
    fenix,
    flake-utils,
    advisory-db,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = nixpkgs.legacyPackages.${system};
      inherit (pkgs) lib;

      craneLib = crane.mkLib pkgs;
      src = craneLib.cleanCargoSource ./.;

      commonArgs = {
        inherit src;
        strictDeps = true;

        buildInputs =
          [
            pkgs.openssl
            pkgs.pkg-config
          ]
          ++ lib.optionals pkgs.stdenv.isDarwin [
            pkgs.libiconv
            pkgs.darwin.apple_sdk.frameworks.Security
            pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
          ];

        DISCORD_TOKEN = ""; # Token will be provided via environment
      };

      cargoArtifacts = craneLib.buildDepsOnly commonArgs;

      ember = craneLib.buildPackage (commonArgs
        // {
          inherit cargoArtifacts;
        });

      # NixOS module for the Ember bot service
      emberModule = {
        config,
        lib,
        pkgs,
        ...
      }: {
        options.services.ember = {
          enable = lib.mkEnableOption "Ember Discord bot";
          tokenFile = lib.mkOption {
            type = lib.types.path;
            description = "File containing the Discord bot token";
          };
          user = lib.mkOption {
            type = lib.types.str;
            default = "ember";
            description = "User account under which the bot runs";
          };
          group = lib.mkOption {
            type = lib.types.str;
            default = "ember";
            description = "Group under which the bot runs";
          };
        };

        config = lib.mkIf config.services.ember.enable {
          users.users.${config.services.ember.user} = {
            isSystemUser = true;
            group = config.services.ember.group;
            description = "Ember Discord bot service user";
          };

          users.groups.${config.services.ember.group} = {};

          systemd.services.ember = {
            description = "Ember Discord Bot";
            wantedBy = ["multi-user.target"];
            after = ["network-online.target"];
            wants = ["network-online.target"];

            serviceConfig = {
              Type = "simple";
              User = config.services.ember.user;
              Group = config.services.ember.group;
              ExecStart = "${ember}/bin/ember";
              Restart = "always";
              RestartSec = "30s";

              # Security hardening
              NoNewPrivileges = true;
              PrivateTmp = true;
              PrivateDevices = true;
              ProtectSystem = "strict";
              ProtectHome = true;
              ReadOnlyDirectories = "/";
              ReadWritePaths = [];
              PrivateUsers = true;

              # Environment setup
              EnvironmentFile = config.services.ember.tokenFile;
            };
          };
        };
      };
    in {
      checks = {
        inherit ember;

        ember-clippy = craneLib.cargoClippy (commonArgs
          // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets -- --deny warnings";
          });

        ember-fmt = craneLib.cargoFmt {
          inherit src;
        };

        ember-audit = craneLib.cargoAudit {
          inherit src advisory-db;
        };

        ember-nextest = craneLib.cargoNextest (commonArgs
          // {
            inherit cargoArtifacts;
            partitions = 1;
            partitionType = "count";
          });
      };

      packages = {
        default = ember;
      };

      nixosModules.default = emberModule;

      apps.default = flake-utils.lib.mkApp {
        drv = ember;
      };

      devShells.default = craneLib.devShell {
        checks = self.checks.${system};

        packages = with pkgs; [
          pkg-config
          openssl
          rust-analyzer
          alejandra
        ];
      };
    });
}
