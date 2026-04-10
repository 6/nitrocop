# nitrocop-config: EnforcedStyle: with_fixed_indentation

def method(
      a,
      ^ Layout/ParameterAlignment: Use one level of indentation for parameters following the first line of a multi-line method definition.
      b)
      ^ Layout/ParameterAlignment: Use one level of indentation for parameters following the first line of a multi-line method definition.
end

def method(a = nil,
           b,
           ^ Layout/ParameterAlignment: Use one level of indentation for parameters following the first line of a multi-line method definition.
           c)
           ^ Layout/ParameterAlignment: Use one level of indentation for parameters following the first line of a multi-line method definition.
end

	def tabbed(a,
		b)
		^ Layout/ParameterAlignment: Use one level of indentation for parameters following the first line of a multi-line method definition.
	end
