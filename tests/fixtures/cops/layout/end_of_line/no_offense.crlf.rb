# nitrocop-config: EnforcedStyle: crlf
# nitrocop-filename: test_windows_1252.rb
# encoding:windows-1252
# frozen_string_literal: false

require "test/unit"

class TestWindows1252 < Test::Unit::TestCase
  def test_stset
    assert_match(/^(\xdf)\1$/i, "\xdf\xdf")
  end
end
