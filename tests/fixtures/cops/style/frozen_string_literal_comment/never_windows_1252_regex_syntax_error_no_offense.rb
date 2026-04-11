# nitrocop-filename: test_windows_1252.rb
# encoding: windows-1252
# frozen_string_literal: false

assert_match(/^(\xdf)\1$/i, "\xdf\xdf")
