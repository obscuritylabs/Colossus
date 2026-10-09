class Colossus < Formula
  desc "Auditable runtime for agent work and durable automation"
  homepage "https://github.com/obscuritylabs/Colossus"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/obscuritylabs/Colossus/releases/download/v0.11.7/colossus-0.11.7-aarch64-apple-darwin.tar.gz"
      sha256 "c33f8fae21d8dba9531a939c2468c9d696ffa64a92c5ee2665c47dce503b31d5"
    else
      url "https://github.com/obscuritylabs/Colossus/releases/download/v0.11.7/colossus-0.11.7-x86_64-apple-darwin.tar.gz"
      sha256 "ad36dd071552501a565d6abfd3d38a2b40ec9c0d9be667b71c1811a55e650b05"
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
