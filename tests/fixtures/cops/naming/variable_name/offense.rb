badVariable = 1
^^^^^^^^^^^ Naming/VariableName: Use snake_case for variable names.

myValue = 2
^^^^^^^ Naming/VariableName: Use snake_case for variable names.

firstName = "John"
^^^^^^^^^ Naming/VariableName: Use snake_case for variable names.

@badVariable = 1
^^^^^^^^^^^^ Naming/VariableName: Use snake_case for variable names.

@myValue = 2
^^^^^^^^ Naming/VariableName: Use snake_case for variable names.

@@badVariable = 1
^^^^^^^^^^^^^ Naming/VariableName: Use snake_case for variable names.

def foo(badParam)
        ^^^^^^^^ Naming/VariableName: Use snake_case for variable names.
end

def bar(ok, badName:)
            ^^^^^^^^ Naming/VariableName: Use snake_case for variable names.
end

firstArg = "foo"
^^^^^^^^ Naming/VariableName: Use snake_case for variable names.
do_something(firstArg)
             ^^^^^^^^ Naming/VariableName: Use snake_case for variable names.

items.each do |itemName|
               ^^^^^^^^ Naming/VariableName: Use snake_case for variable names.
end

[1, 2].map { |numVal| numVal }
              ^^^^^^ Naming/VariableName: Use snake_case for variable names.
                      ^^^^^^ Naming/VariableName: Use snake_case for variable names.

_myLocal = 1
^^^^^^^^ Naming/VariableName: Use snake_case for variable names.

badName, ok = [1, 2]
^^^^^^^ Naming/VariableName: Use snake_case for variable names.

badCompound ||= 1
^^^^^^^^^^^ Naming/VariableName: Use snake_case for variable names.

badAnd &&= true
^^^^^^ Naming/VariableName: Use snake_case for variable names.

badOp += 1
^^^^^ Naming/VariableName: Use snake_case for variable names.

@badIvar ||= compute
^^^^^^^^^ Naming/VariableName: Use snake_case for variable names.

@badIvarAnd &&= true
^^^^^^^^^^^ Naming/VariableName: Use snake_case for variable names.

@badIvarOp += 1
^^^^^^^^^^ Naming/VariableName: Use snake_case for variable names.

@@badCvar += 1
^^^^^^^^^^ Naming/VariableName: Use snake_case for variable names.

@@badCvarOr ||= 0
^^^^^^^^^^^ Naming/VariableName: Use snake_case for variable names.

叫聲 = "meow"
^^ Naming/VariableName: Use snake_case for variable names.

for badItem in items; end
    ^^^^^^^ Naming/VariableName: Use snake_case for variable names.

begin
  something
rescue => badError
          ^^^^^^^^ Naming/VariableName: Use snake_case for variable names.
end

begin
  something
rescue StandardError => @badIvarError
                        ^^^^^^^^^^^^^ Naming/VariableName: Use snake_case for variable names.
end

begin
  something
rescue => @@badCvarErr
          ^^^^^^^^^^^^ Naming/VariableName: Use snake_case for variable names.
end

items.each do |ok, (badInner, alsoBAD)|
                    ^^^^^^^^ Naming/VariableName: Use snake_case for variable names.
                              ^^^^^^^ Naming/VariableName: Use snake_case for variable names.
end

def destructured((good_ok, badNested))
                           ^^^^^^^^^ Naming/VariableName: Use snake_case for variable names.
end

[1].each do |*_, postParam|
                 ^^^^^^^^^ Naming/VariableName: Use snake_case for variable names.
end
