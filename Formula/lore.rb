# Homebrew formula for the lore CLI.
#
# To install from this tap:
#   brew tap lore-dev/lore
#   brew install lore
#
# To install directly from the formula file (development):
#   brew install --formula ./Formula/lore.rb

class Lore < Formula
  desc "Local documentation server for AI coding assistants"
  homepage "https://github.com/lore-dev/lore"
  version "0.1.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/lore-dev/lore/releases/download/v#{version}/lore-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "PLACEHOLDER_AARCH64_MACOS_SHA256"
    end

    on_intel do
      url "https://github.com/lore-dev/lore/releases/download/v#{version}/lore-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "PLACEHOLDER_X86_64_MACOS_SHA256"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/lore-dev/lore/releases/download/v#{version}/lore-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "PLACEHOLDER_AARCH64_LINUX_SHA256"
    end

    on_intel do
      url "https://github.com/lore-dev/lore/releases/download/v#{version}/lore-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "PLACEHOLDER_X86_64_LINUX_SHA256"
    end
  end

  def install
    bin.install "lore"
  end

  def caveats
    <<~EOS
      The first time lore downloads a package it will fetch the bge-small-en-v1.5
      embedding model (~130 MB) and cache it in:
        #{Dir.home}/.cache/lore/models/

      To get started, add a documentation package:
        lore add tokio

      Then configure your AI assistant to use the MCP server:
        lore mcp
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/lore --version")
  end
end
