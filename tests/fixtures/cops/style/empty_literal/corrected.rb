# frozen_string_literal: false
a = []

h = {}

s = ''

values = Hash.new { |hash, key| hash[key] = {} }

@token_regexps       = Hash.new { |h,k| h[ k ] = {} }

queues = Array.new(n) {|i| [] }
