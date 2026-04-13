# frozen_string_literal: true

module Nitrocop
  VERSION = "0.0.1.pre11"

  # Returns the path to the precompiled nitrocop binary, or nil if
  # no binary is bundled (e.g. the base/fallback gem on an unsupported platform).
  def self.executable
    bin = File.expand_path("../libexec/nitrocop", __dir__)
    bin if File.file?(bin) && File.executable?(bin)
  end
end
