class Colossus < Formula
  desc "Auditable runtime for agent work and durable automation"
  homepage "https://github.com/obscuritylabs/Colossus"
  version "0.10.5"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/obscuritylabs/Colossus/releases/download/v0.10.5/colossus-0.10.5-aarch64-apple-darwin.tar.gz"
      sha256 "115aaa6dffb1647b3e6e00757b5b136eb710975bad04ac72409e3ed60f012856"
    else
      url "https://github.com/obscuritylabs/Colossus/releases/download/v0.10.5/colossus-0.10.5-x86_64-apple-darwin.tar.gz"
      sha256 "6d768d4204bf8854eb7edfc4c67a528501b0c6d75f4e586f3186b0e36dcbc9dd"
    end
  end

  def install
    libexec.install "colossus"
    bin.write_env_script libexec/"colossus", COLOSSUS_INSTALLER_KIND: "homebrew"
  end

  test do
    assert_equal "colossus #{version}", shell_output("#{bin}/colossus --version").strip
  end
end
