use unimock::private::lib::{vec, Vec};
use unimock::*;

#[derive(Eq, PartialEq)]
pub struct Data(Vec<u32>);

#[unimock(api=TestMock)]
trait Test {
    fn f(&self, a: u32, b: Data) -> i32;
}

#[test]
fn test_eq() {
    let u = Unimock::new((
        TestMock::f
            .each_call(matching!(_, eq!(&Data(vec![1]))))
            .returns(1),
        TestMock::f
            .each_call(matching!(_, eq!(&Data(vec![2]))))
            .returns(2),
        TestMock::f
            .each_call(matching!(_, eq!(&Data(vec![3]))))
            .returns(3),
        TestMock::f
            .each_call(matching!((a, ne!(&Data(vec![1337]))) if a != &0 ))
            .returns(42),
        TestMock::f.each_call(matching!(_, _)).returns(1337),
    ));

    assert_eq!(1, <Unimock as Test>::f(&u, 0, Data(vec![1])));
    assert_eq!(2, <Unimock as Test>::f(&u, 0, Data(vec![2])));
    assert_eq!(3, <Unimock as Test>::f(&u, 0, Data(vec![3])));
    assert_eq!(1, <Unimock as Test>::f(&u, 0, Data(vec![1])));
    assert_eq!(1337, <Unimock as Test>::f(&u, 0, Data(vec![42])));
    assert_eq!(42, <Unimock as Test>::f(&u, 1, Data(vec![42])));
    assert_eq!(1337, <Unimock as Test>::f(&u, 0, Data(vec![1337])));
}

#[test]
fn eq_or() {
    let u = Unimock::new((
        TestMock::f
            .each_call(matching!(
                (_, eq!(&Data(vec![42]))) | (_, eq!(&Data(vec![1337])))
            ))
            .returns(42),
        TestMock::f.each_call(matching!(_, _)).returns(0),
    ));

    assert_eq!(42, <Unimock as Test>::f(&u, 0, Data(vec![42])));
    assert_eq!(42, <Unimock as Test>::f(&u, 0, Data(vec![1337])));
    assert_eq!(0, <Unimock as Test>::f(&u, 0, Data(vec![123])));
}
