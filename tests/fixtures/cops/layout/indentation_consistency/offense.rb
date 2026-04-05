def foo
  x = 1
    y = 2
    ^^^ Layout/IndentationConsistency: Inconsistent indentation detected.
end

class Bar
  a = 1
      b = 2
      ^^^ Layout/IndentationConsistency: Inconsistent indentation detected.
end

module Baz
  c = 1
        d = 2
        ^^^ Layout/IndentationConsistency: Inconsistent indentation detected.
end

if cond
 func
  func
  ^^^^ Layout/IndentationConsistency: Inconsistent indentation detected.
end

if cond
  func1
else
 func2
  func2
  ^^^^^ Layout/IndentationConsistency: Inconsistent indentation detected.
end

unless cond
 func
  func
  ^^^^ Layout/IndentationConsistency: Inconsistent indentation detected.
end

case a
when b
 c
    d
    ^ Layout/IndentationConsistency: Inconsistent indentation detected.
end

while cond
 func
  func
  ^^^^ Layout/IndentationConsistency: Inconsistent indentation detected.
end

until cond
 func
  func
  ^^^^ Layout/IndentationConsistency: Inconsistent indentation detected.
end

for var in 1..10
 func
func
^^^^ Layout/IndentationConsistency: Inconsistent indentation detected.
end

begin
 func1
   func2
   ^^^^^ Layout/IndentationConsistency: Inconsistent indentation detected.
end

agent.measure_block("test") do
  ActiveSupport::Notifications.instrument("deliver.action_mailer", {mailer: "Mailer"}) do line = __LINE__
    sleep 0.01
    ^^^^^^^^^^ Layout/IndentationConsistency: Inconsistent indentation detected.
  end
end

def foo
  a
    b
    ^ Layout/IndentationConsistency: Inconsistent indentation detected.
rescue
  c
end

begin
	foo
rescue Exception => ex
	bar
  ex
  ^^ Layout/IndentationConsistency: Inconsistent indentation detected.
end

class A
  class << self
    private
      def first_block_start(language, parent_block, line_number, string, offset, maximum_offset = nil)
      ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/IndentationConsistency: Inconsistent indentation detected.
      end
  end
end

require 'ostruct'

 module ClinicFinder
 ^^^^^^^^^^^^^^^^^^^ Layout/IndentationConsistency: Inconsistent indentation detected.
  module Modules
    module GestationHelper; end
  end
end

case type
when :po
  po_paths = 1
else :mo
  mo_paths = 2
  ^^^^^^^^^^^^ Layout/IndentationConsistency: Inconsistent indentation detected.
end

object = Class.new do
           private

 def hello_world; end
 ^^^^^^^^^^^^^^^^^^^^ Layout/IndentationConsistency: Inconsistent indentation detected.
          end.new

included do
  private
    def affiliate_basis_points_must_fall_in_an_acceptable_range
    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/IndentationConsistency: Inconsistent indentation detected.
    end
end

-> do
  limit = -1
   v = 0
   ^^^^^ Layout/IndentationConsistency: Inconsistent indentation detected.
end

->{   1
 2
 ^ Layout/IndentationConsistency: Inconsistent indentation detected.
}

BEGIN {
  require 'pathname'
    load_path = 'lib'
    ^^^^^^^^^^^^^^^^^ Layout/IndentationConsistency: Inconsistent indentation detected.
}

x = (a = 1
b = 2
^ Layout/IndentationConsistency: Inconsistent indentation detected.
c = 3)
^^^^^ Layout/IndentationConsistency: Inconsistent indentation detected.
