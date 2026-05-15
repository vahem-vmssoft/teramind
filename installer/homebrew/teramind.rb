class Teramind < Formula
  desc "Local-first AI knowledge consolidation substrate for coding agents"
  homepage "https://get.teramind.dev"
  version "0.1.0"   # bumped by release CI before publishing to tap
  license "Apache-2.0"

  if Hardware::CPU.arm?
    url "https://get.teramind.dev/#{version}/teramind-#{version}-aarch64-apple-darwin.tar.gz"
    sha256 "REPLACE_WITH_RELEASE_SUM"
  else
    url "https://get.teramind.dev/#{version}/teramind-#{version}-x86_64-apple-darwin.tar.gz"
    sha256 "REPLACE_WITH_RELEASE_SUM"
  end

  def install
    bin.install "teramind", "teramindd", "teramind-hook", "teramind-mcp"
    (libexec/"plugins/claude").install Dir["plugins-claude/*"]
  end

  test do
    assert_match(/teramind/, shell_output("#{bin}/teramind --version"))
  end
end
