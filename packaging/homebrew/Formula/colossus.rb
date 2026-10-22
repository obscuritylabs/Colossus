class Colossus < Formula
  desc "Auditable runtime for agent work and durable automation"
  homepage "https://github.com/obscuritylabs/Colossus"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/obscuritylabs/Colossus/releases/download/v0.11.10/colossus-0.11.10-aarch64-apple-darwin.tar.gz"
      sha256 "a0c5a8a5f9e75a31acd2d3ca13120cdd42a898fe755a1bc3f712aebec50e0321"
    else
      url "https://github.com/obscuritylabs/Colossus/releases/download/v0.11.10/colossus-0.11.10-x86_64-apple-darwin.tar.gz"
      sha256 "3293dbdf5dbdcae1b00d5cdffa599525868a7f2609ab505e1ee7ecf3c44a09fd"
    end
  end

  def install
    libexec.install "colossus"
    if (buildpath/"tools/rg").exist?
      libexec.install "tools/rg" => "rg"
      (share/"licenses/colossus/ripgrep").install "tools/COPYING", "tools/LICENSE-MIT", "tools/UNLICENSE"
    end
    bundled_rg = (libexec/"rg").exist? ? "1" : "0"
    env = { COLOSSUS_INSTALLER_KIND: "homebrew", COLOSSUS_BUNDLED_RIPGREP: bundled_rg }
    (bin/"colossus").write_env_script libexec/"colossus", env
  end

  test do
    assert_equal "colossus #{version}", shell_output("#{bin}/colossus --version").strip
    if (libexec/"rg").exist?
      assert_match "ripgrep 15.2.0", shell_output("#{libexec}/rg --version")
    end
  end
end
