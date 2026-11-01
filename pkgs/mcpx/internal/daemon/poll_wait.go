package daemon

import (
	"errors"
	"fmt"
	"math"
	"strconv"
	"time"
)

// pollWait applies the endpoint's resolved setting as the maximum wait.
func pollWait(raw string, ceiling time.Duration) (time.Duration, error) {
	ms, err := strconv.ParseInt(raw, 10, 64)
	if errors.Is(err, strconv.ErrRange) {
		return 0, fmt.Errorf("waitMs is out of range")
	}
	if err != nil || ms == 0 {
		return ceiling, nil
	}
	if ms < 0 || ms > math.MaxInt64/int64(time.Millisecond) {
		return 0, fmt.Errorf("waitMs is out of range")
	}
	wait := time.Duration(ms) * time.Millisecond
	if wait > ceiling {
		return ceiling, nil
	}
	return wait, nil
}
