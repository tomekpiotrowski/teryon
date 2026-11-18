import { addressFromFragment, addressFromStorage } from './service-address';

describe('addressFromFragment', () => {
  it('reads the port and the token', () => {
    expect(addressFromFragment('#port=7317&token=abc')).toEqual({ port: 7317, token: 'abc' });
  });

  it('reads them in either order and without the hash', () => {
    expect(addressFromFragment('token=abc&port=7318')).toEqual({ port: 7318, token: 'abc' });
  });

  it.each([
    [''],
    ['#'],
    ['#port=7317'],
    ['#token=abc'],
    ['#port=&token=abc'],
    ['#port=seven&token=abc'],
    ['#port=0&token=abc'],
    ['#port=70000&token=abc'],
    ['#port=73.5&token=abc'],
    ['#port=7317&token='],
  ])('finds nothing in %j', (fragment) => {
    expect(addressFromFragment(fragment)).toBeNull();
  });
});

describe('addressFromStorage', () => {
  it('reads an address kept earlier', () => {
    expect(addressFromStorage('{"port":7317,"token":"abc"}')).toEqual({ port: 7317, token: 'abc' });
  });

  it.each([[null], [''], ['not json'], ['{}'], ['{"port":"7317","token":"abc"}'], ['{"port":7317}'], ['[]']])(
    'finds nothing in %j',
    (text) => {
      expect(addressFromStorage(text)).toBeNull();
    },
  );
});
