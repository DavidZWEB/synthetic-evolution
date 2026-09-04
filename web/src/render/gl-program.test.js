/** WebGL shader/program cleanup regressions. */

import assert from 'node:assert/strict';
import test from 'node:test';

import { createProgram } from './gl-program.js';

function fakeGl({ link = true } = {}) {
  const deletedShaders = [];
  const detachedShaders = [];
  const deletedPrograms = [];
  const gl = {
    VERTEX_SHADER: 1,
    FRAGMENT_SHADER: 2,
    COMPILE_STATUS: 3,
    LINK_STATUS: 4,
    createShader: (type) => ({ type }),
    shaderSource() {},
    compileShader() {},
    getShaderParameter: () => true,
    getShaderInfoLog: () => '',
    deleteShader: (shader) => deletedShaders.push(shader),
    createProgram: () => ({ id: 1 }),
    attachShader() {},
    linkProgram() {},
    getProgramParameter: () => link,
    getProgramInfoLog: () => 'link failed',
    detachShader: (_program, shader) => detachedShaders.push(shader),
    deleteProgram: (program) => deletedPrograms.push(program),
  };
  return { gl, deletedShaders, detachedShaders, deletedPrograms };
}

test('linked programs release their transient shaders', () => {
  const fake = fakeGl();
  const program = createProgram(fake.gl, 'vertex', 'fragment');
  assert.ok(program);
  assert.equal(fake.deletedShaders.length, 2);
  assert.equal(fake.detachedShaders.length, 2);
  assert.equal(fake.deletedPrograms.length, 0);
});

test('a link failure releases the program and both shaders', () => {
  const fake = fakeGl({ link: false });
  assert.throws(() => createProgram(fake.gl, 'vertex', 'fragment'), /link failed/);
  assert.equal(fake.deletedShaders.length, 2);
  assert.equal(fake.detachedShaders.length, 2);
  assert.equal(fake.deletedPrograms.length, 1);
});
