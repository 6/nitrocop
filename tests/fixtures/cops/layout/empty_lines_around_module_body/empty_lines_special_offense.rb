module Example
  include Something
  def work; end
^ Layout/EmptyLinesAroundModuleBody: Empty line missing before first def definition.
end
^ Layout/EmptyLinesAroundModuleBody: Empty line missing at module body end.

module BlankBeginning

^ Layout/EmptyLinesAroundModuleBody: Extra empty line detected at module body beginning.
  include Something
  def work; end
^ Layout/EmptyLinesAroundModuleBody: Empty line missing before first def definition.

end

module ModuleFunctionSpecial
  module_function

  def work; end
end
^ Layout/EmptyLinesAroundModuleBody: Empty line missing at module body end.

module WhitespaceBeforeComment
  extend self
  
  # docs
^ Layout/EmptyLinesAroundModuleBody: Empty line missing before first def definition.
  def work; end
end
^ Layout/EmptyLinesAroundModuleBody: Empty line missing at module body end.
