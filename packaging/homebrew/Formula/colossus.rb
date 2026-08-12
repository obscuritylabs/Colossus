class Colossus < Formula
  desc "Auditable runtime for agent work and durable automation"
  homepage "https://github.com/obscuritylabs/Colossus"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/obscuritylabs/Colossus/releases/download/v0.10.7/colossus-0.10.7-aarch64-apple-darwin.tar.gz"
      sha256 "035d66323a2d07026266838a0799077e16e16e1f3d8d1f803a9f5f81ee2b7d5a"
    else
      url "https://github.com/obscuritylabs/Colossus/releases/download/v0.10.7/colossus-0.10.7-x86_64-apple-darwin.tar.gz"
      sha256 "f5ccbce846efe485e411dd9dd7bc878d847719fc6012e001f84ea8c6142b68cc"
    end
  end

  def install
    libexec.install "colossus"
    (bin/"colossus").write_env_script libexec/"colossus", COLOSSUS_INSTALLER_KIND: "homebrew"
  end

  test do
    assert_equal "colossus #{version}", shell_output("#{bin}/colossus --version").strip
  end
end
