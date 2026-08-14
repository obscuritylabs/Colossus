class Colossus < Formula
  desc "Auditable runtime for agent work and durable automation"
  homepage "https://github.com/obscuritylabs/Colossus"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/obscuritylabs/Colossus/releases/download/v0.10.8/colossus-0.10.8-aarch64-apple-darwin.tar.gz"
      sha256 "be2f6b6f2274a7e1b4db4b9856d1c4627f938b6afc77cab2ffc344bb5bef39b0"
    else
      url "https://github.com/obscuritylabs/Colossus/releases/download/v0.10.8/colossus-0.10.8-x86_64-apple-darwin.tar.gz"
      sha256 "6d6b5897bbd87afe9646dac8b34bde89a6ea5a50706bd376532dc8cc9a8ba25c"
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
