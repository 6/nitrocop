# nitrocop-filename: header.xls.builder
# frozen_string_literal: true
^ Style/FrozenStringLiteralComment: Unnecessary frozen string literal comment.

xml.instruct!
xml.Workbook "xmlns:x" => "urn:schemas-microsoft-com:office:excel",
             "xmlns:ss" => "urn:schemas-microsoft-com:office:spreadsheet",
             "xmlns:html" => "http://www.w3.org/TR/REC-html40",
             "xmlns" => "urn:schemas-microsoft-com:office:spreadsheet",
             "xmlns:o" => "urn:schemas-microsoft-com:office:office" do
  xml << yield
end
