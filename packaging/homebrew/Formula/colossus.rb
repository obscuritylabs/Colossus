class Colossus < Formula
  desc "Auditable runtime for agent work and durable automation"
  homepage "https://github.com/obscuritylabs/Colossus"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/obscuritylabs/Colossus/releases/download/v0.10.9/colossus-0.10.9-aarch64-apple-darwin.tar.gz"
      sha256 "9eb7f10e7cd345c20b3c63b51484d3749bd76ae2bd470fa6756781783f6742d9"
    else
      url "https://github.com/obscuritylabs/Colossus/releases/download/v0.10.9/colossus-0.10.9-x86_64-apple-darwin.tar.gz"
      sha256 "5b9a57ccd2fdc5efe8c41b5e24594daacf9dfafa38bce3a3f240f7a07dd272fa"
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
