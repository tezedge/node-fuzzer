import json
from pathlib import Path
from html.parser import HTMLParser
from enum import Enum

class Context(Enum):
    Unknown = 0
    lineNum = 1
    lineNoCov = 2
    lineCov = 3
    headerItem = 4
    headerLines = 5
    headerCovTableEntryHit = 6
    headerCovTableEntryTotal = 7
    headerDate = 8
    headerDateValue = 9
    headerCovTableEntryHitDone = 10
    headerCovTableEntryTotalDone = 11

class MyHTMLParser(HTMLParser):
    def __init__(self, header, lines):
        super().__init__()
        self.current = Context.Unknown
        self.source = False
        self.end = False
        self.header = header
        self.lines = lines
        self.line = dict()

    def handle_endtag(self, tag):
        if self.end:
            return

        if tag == 'pre':
            if self.line:
                self.lines.append(self.line)
            if self.source:
                self.end = True

        if tag == 'span':
            self.current = Context.Unknown

    def handle_starttag(self, tag, attrs):
        if self.end:
            return

        if not self.source:
            if tag == 'td':
                attrs = dict(attrs)

                try:
                    if attrs['class'] == 'headerItem':
                        self.current = Context.headerItem

                    if self.current == Context.headerCovTableEntryHitDone and \
                        attrs['class'] == 'headerCovTableEntry':
                        self.current = Context.headerCovTableEntryTotal

                    if self.current == Context.headerLines and \
                        attrs['class'] == 'headerCovTableEntry':
                        self.current = Context.headerCovTableEntryHit

                    if self.current == Context.headerDate and \
                        attrs['class'] == 'headerValue':
                        self.current = Context.headerDateValue

                except:
                    pass

        if tag == 'pre':
            attrs = dict(attrs)
            try:
                if attrs['class'] == 'source':
                    self.source = True

            except:
                pass

        if self.source and tag == 'span':
            attrs = dict(attrs)

            try:
                if attrs['class'] == 'lineNum':
                    self.current = Context.lineNum

                if attrs['class'] == 'lineNoCov':
                    self.current = Context.lineNoCov

                if attrs['class'] == 'lineCov':
                    self.current = Context.lineCov

            except:
                pass

    def handle_data(self, data):
        if self.end:
            return

        if self.source:
            if self.current == Context.Unknown:
                if ':' in data:
                    self.line['line'] = data.split(':')[1]

            if self.current == Context.lineNum:
                if self.line:
                    self.lines.append(self.line)

                self.line = dict()
                self.line['lineNum'] = data

            if self.current == Context.lineNoCov:
                self.line['class'] = 'linePartCov'
                self.line['hits'] = '0'
                self.line['order'] = '0'
                self.line['possible_hits'] = '1'
                self.line['line'] = data.split(':')[1]

            if self.current == Context.lineCov:
                self.line['class'] = 'linePartCov'
                self.line['hits'] = '1'
                self.line['order'] = '0'
                self.line['possible_hits'] = '1'
                self.line['line'] = data.split(':')[1]
        else:
            if self.current == Context.headerItem:
                if data == 'Lines:':
                    self.current = Context.headerLines

                if data == 'Date:':
                    self.current = Context.headerDate

            if self.current == Context.headerCovTableEntryHit:
                self.header['covered'] = int(data)
                self.current = Context.headerCovTableEntryHitDone

            if self.current == Context.headerCovTableEntryTotal:
                self.header['instrumented'] = int(data)
                self.current = Context.headerCovTableEntryTotalDone

            if self.current == Context.headerDateValue:
                self.header['date'] = data
                self.current = Context.Unknown

html_template = """
<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN">
<html>
<head>
  <meta charset="utf-8">
  <title id="window-title">???</title>
  <link rel="stylesheet" type="text/css" href="../data/bcov.css"/>
</head>
<script type="text/javascript" src="data/js/jquery.min.js"></script>
<script type="text/javascript" src="data/js/handlebars.js"></script>
<script type="text/javascript" src="data/js/kcov.js"></script>

<body>

<table width="100%" border="0" cellspacing="0" cellpadding="0">
  <tr><td class="title">Coverage Report</td></tr>
  <tr><td class="ruler"><img src="data/glass.png" width="3" height="3" alt=""/></td></tr>
  <tr>
    <td width="100%">
      <table cellpadding="1" border="0" width="100%">
        <tr id="command">
          <td class="headerItem" width="20%">Command:</td>
          <td id="header-command" class="headerValue" width="60%" colspan=4>???</td>
          <td><span class="lineNumLegend">Line number</span></td>
        </tr>
        <tr>
          <td class="headerItem" width="20%">Date: </td>
          <td id="header-date" class="headerValue" width="20%" colspan=2></td>
          <td class="headerItem" width="20%">Instrumented lines:</td>
          <td id="header-instrumented" class="headerValue" width="20%">???</td>
          <td><span class="coverHitsLegend">Hits</span></td>
        </tr>
        <tr>
          <td class="headerItem" width="20%">Code covered:</td>
          <td id="header-percent-covered" width="20%" colspan=2>???</td>
          <td class="headerItem" width="20%">Executed lines:</td>
          <td id="header-covered" class="headerValue" width="20%">???</td>
          <td><span class="orderNumLegend">Order</span></td>
        </tr>
      </table>
    </td>
  </tr>
  <tr><td class="ruler"><img src="data/glass.png" width="3" height="3" alt=""/></td></tr>
</table>

<script id="lines-template" type="text/x-handlebars-template">
<pre class="source" id="main-data">
{{#each lines}}
{{#ifCond possible_hits "==" "0"}}
<source-line><span class="lineNum">{{lineNum}}</span><span class="coverHits">{{hits}} </span><span class="{{class}}"> {{line}}</span></span><span class="orderNum"> {{order}}</span></source-line>
{{else}}
 {{#exists possible_hits}}
<source-line><span class="lineNum">{{lineNum}}</span><span class="coverHits">{{hits}} / {{possible_hits}}</span><span class="{{class}}"> {{line}}</span><span class="orderNum"> {{order}}</span></source-line>
 {{else}}
   {{#exists class}}
<source-line><span class="lineNum">{{lineNum}}</span><span class="coverHits">&nbsp;</span><span class="{{class}}"> {{line}}</span></span><span class="orderNum">&nbsp;</span></source-line>
   {{else}}
<source-line><span class="lineNum">{{lineNum}}</span><span class="coverHits">&nbsp;</span><span class="{{class}}"> {{line}}</span><span class="orderNum">&nbsp;</span></source-line>
   {{/exists}}
 {{/exists}}
{{/ifCond}}
{{/each}}
</pre>
</script>
<div id="lines-placeholder"></div>

<table width="100%" border="0" cellspacing="0" cellpadding="0" id="merged-data">
  <tr><td class="ruler"><img src="data/amber.png" width="3" height="3" alt=""/></td></tr>
  <tr><td class="versionInfo">Generated by: <a href="http://simonkagstrom.github.com/kcov/index.html">Kcov</a></td></tr>
</table>
</body>
</html>
"""

path = '/coverage/develop/.fuzzing.latest/custom-fuzzer/'

for path in Path(path).rglob('*.gcov.html'):
    file = str(path)
    header = { 'command': 'TODO' }
    lines = []
    parser = MyHTMLParser(header, lines)

    with open(file,'r') as input_file:
        parser.feed(input_file.read())

    file = file[:-len('.gcov.html')]

    with open(f'{file}.js', 'w') as js_file:
        js_file.write((
            f'var data = {{lines: {json.dumps(lines)}}};'
            f'var percent_low = 25;var percent_high = 75;'
            f'var header = {json.dumps(header)};'
            f'var merged_data = [];'
        ))

    with open(f'{file}.kcov.html', 'w') as html_file:
        file = path.name[:-len('.gcov.html')]
        html_file.write(
            f'<script type="text/javascript" src="{file}.js"></script>'
        )
        html_file.write(html_template)


