class Winctl2rust < Formula
  desc "Pure-Rust Iced GUI, CLI, and policy-gated MCP server for non-hooking Windows control & offline EDID/OpenType forensics"
  homepage "https://guap.ca/winctl2rust/"
  version "0.1.0"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/adpaolucci/winctl2rust/releases/download/v0.1.0/winctl2rust-aarch64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    else
      url "https://github.com/adpaolucci/winctl2rust/releases/download/v0.1.0/winctl2rust-x86_64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  on_linux do
    if Hardware::CPU.intel?
      url "https://github.com/adpaolucci/winctl2rust/releases/download/v0.1.0/winctl2rust-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  def install
    bin.install "winctl2rust"
    bin.install "winctl2rust-iced"
    bin.install "winctl2rust-mcp"
    doc.install "README.md", "CHANGELOG.md"
    doc.install "SBOM-crates.txt" if File.exist?("SBOM-crates.txt")
    prefix.install "LICENSE", "NOTICE"
  end

  test do
    assert_match "winctl2rust", shell_output("#{bin}/winctl2rust --help")
    assert_match "winctl2rust-iced", shell_output("#{bin}/winctl2rust-iced --help")
  end
end
