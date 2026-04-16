# nitrocop-config: EnforcedStyleAlignWith: relative_to_receiver

result = if condition
           value_a
           ^^^^^^^ Layout/IndentationWidth: Use 1 (not 5) tabs for indentation.
         end

case resource_name
when Proc
    resource_name.call(controller)
    ^^^^^^^^^^^^^ Layout/IndentationWidth: Use 1 (not 2) tabs for indentation.
else
  default_name
end
