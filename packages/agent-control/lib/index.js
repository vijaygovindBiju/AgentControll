'use strict';

const platform = require('./platform');
const downloader = require('./downloader');
const runner = require('./runner');

module.exports = {
  ...platform,
  ...downloader,
  ...runner,
};
