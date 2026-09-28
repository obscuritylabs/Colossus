class Colossus < Formula
  desc "Auditable runtime for agent work and durable automation"
  homepage "https://github.com/obscuritylabs/Colossus"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/obscuritylabs/Colossus/releases/download/v0.11.0/colossus-0.11.0-aarch64-apple-darwin.tar.gz"
      sha256 "c5140fa28173641838069e4ec331ef3848c8f3a4f3149b174e09380719e75b9a"
    else
      url "https://github.com/obscuritylabs/Colossus/releases/download/v0.11.0/colossus-0.11.0-x86_64-apple-darwin.tar.gz"
      sha256 "c274017f83a11b47123e80471417548029123b245ae04b0ae4386b78c2b07478"
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
