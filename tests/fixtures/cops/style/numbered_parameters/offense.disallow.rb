# nitrocop-config: EnforcedStyle: disallow
class Schema
  def to_h
    super { [_1.name, _1] }
    ^^^^^^^^^^^^^^^^^^^^^^^ Style/NumberedParameters: Avoid using numbered parameters.
  end

  def serialize(row)
    super(row) { _1.to_s }
    ^^^^^^^^^^^^^^^^^^^^^^ Style/NumberedParameters: Avoid using numbered parameters.
  end
end
