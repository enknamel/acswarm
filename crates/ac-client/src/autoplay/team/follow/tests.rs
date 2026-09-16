use super::*;

#[test]
fn a_follower_strays_no_further_than_twice_its_distance() {
    assert_eq!(follow_break(4.0), 10.0);
    assert_eq!(follow_break(8.0), 16.0);
}
